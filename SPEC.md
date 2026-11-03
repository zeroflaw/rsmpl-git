# RSmPL-Git Specification

**Version:** 0.1-draft  
**Status:** Experimental proposal  
**Primary audience:** implementers of LLM-centric patch generation, semantic refactoring engines, code-review tooling, and Rust-aware patch application systems  
**File extension suggestion:** `.rsmpl` 
**Canonical document name:** `specification.md`

## Agent contract (one screen)

Five invariants; everything below is their spelling:

1. **Original coordinates** (10, D.3): every `-old,len` range cites the
   pre-patch file. Read the file, cite spans, author the patch, `--check`
   it. Stale ranges either re-anchor with `drift` evidence or fail loud
   (10.8) — never silently re-based.
2. **Diet-canonical form** (7.1, 9.3, 15.8, D.4, D.5): terse spellings
   (`- fn foo`, `# sweep`, inferred `E T P V Q G W I N`) are canonical;
   long forms parse but canonicalise away. `diffrs` emits the cheapest
   *trustworthy* view, never merely the cheapest bytes (D3 precision
   guards).
3. **Fail-loud** (2.6, 18.6, 25): unprovable sites stay ordinary or fail
   with a machine `repair` — never a guess. Every patch-input failure
   carries a verbatim-appliable `repair` (25.9, D.2); `null` only on
   environmental IO.
4. **Evidence fields** (D.2, 24.6): every success reports `version`
   (one `rsmpl_patch::REPORT_VERSION` number across all binaries),
   `checked: {lowering, semantics, build}`, per-rule `semantics`
   (`instantiated|postcondition|reparse-only|not-applicable`) computed
   by a check that can fail at the reported `spans`, assertable
   `sites`/`binding_meta`/`span_edits`/`follow_skipped_details`/
   `orphans`. No success field is true by construction (D2).
5. **Cycle equalities** (D.5): semantic → native → semantic and native
   → semantic → native round-trip under stated equalities
   (`S″ == S′` canonical fixed point, tree-lossless natives); every
   remaining divergence is named, owned, and pinned — never silent.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Design Goals](#2-design-goals)
3. [Relationship to Git Patches](#3-relationship-to-git-patches)
4. [Relationship to Coccinelle SmPL](#4-relationship-to-coccinelle-smpl)
5. [Core Concepts](#5-core-concepts)
6. [Patch File Structure](#6-patch-file-structure)
7. [RSmPL Hunk Detection](#7-rsmpl-hunk-detection)
8. [RSmPL Hunk Header Syntax](#8-rsmpl-hunk-header-syntax)
9. [RSmPL Directive Lines](#9-rsmpl-directive-lines)
10. [Line Range Semantics](#10-line-range-semantics)
11. [Ordinary Git Hunks](#11-ordinary-git-hunks)
12. [RSmPL Hunk Body Syntax](#12-rsmpl-hunk-body-syntax)
13. [Rust Pattern Language](#13-rust-pattern-language)
14. [Function Item Matching](#14-function-item-matching)
15. [Metavariables](#15-metavariables)
16. [Ellipsis Semantics](#16-ellipsis-semantics)
17. [Matching Semantics](#17-matching-semantics)
18. [Rewrite Semantics](#18-rewrite-semantics)
19. [Semantic Symbol Matching](#19-semantic-symbol-matching)
20. [Moving Code](#20-moving-code)
21. [Macros](#21-macros)
22. [Formatting and Trivia Preservation](#22-formatting-and-trivia-preservation)
23. [Application Order](#23-application-order)
24. [Validation Rules](#24-validation-rules)
25. [Conflict and Error Model](#25-conflict-and-error-model)
26. [Lowering to a Concrete Git Diff](#26-lowering-to-a-concrete-git-diff)
27. [Security and Safety Considerations](#27-security-and-safety-considerations)
28. [Recommended LLM Emission Style](#28-recommended-llm-emission-style)
29. [Examples](#29-examples)
30. [Formal Grammar Sketch](#30-formal-grammar-sketch)
31. [Implementation Notes](#31-implementation-notes)
32. [Non-Goals](#32-non-goals)
33. [Versioning](#33-versioning)
34. [Glossary](#34-glossary)

---

## 1. Overview

RSmPL-Git is a patch format that combines:

1. **ordinary Git unified-diff hunks** for normal text edits; and
2. **RSmPL semantic hunks** for Rust AST-aware transformations.

RSmPL means **Rust Semantic Patch Language**. It is inspired by the Coccinelle SmPL model: write code-like patterns with metavariables and patch-style `-` / `+` edits. Unlike normal text patches, RSmPL hunks are applied by matching Rust syntax or Rust semantic information inside a declared file and line range.

The core idea is:

```text
normal Git patch syntax
+ compact Rust semantic rewrite directives
+ mandatory file/range scoping
```

An RSmPL-Git patch should look as close as possible to a normal Git patch. The file path and line range come from standard Git diff structure. Semantic behavior is enabled using compact directive lines immediately after a hunk header.

A minimal RSmPL hunk looks like this (diet form: single-path header,
elided new side — the `+new,len` side is ignored for RSmPL hunks, Section
10.4; a directive-free rename needs only `rule=`, Section 7.1):

```diff
diff --git src/lib.rs
@@ -10,7 @@ rule=rename_foo
- fn foo
+ fn bar
```

A normal Git hunk remains normal:

```diff
@@ -1,4 +1,5 @@
 mod old;
+mod new;

 fn main() {}
 }
```

An RSmPL-aware engine applies both kinds of hunks and can emit a final ordinary Git diff.

---

## 2. Design Goals

### 2.1 Token efficiency

The format is intended for LLM-generated patches. LLMs should not need to restate large unchanged functions, modules, or moved code blocks when a semantic description is enough.

For example, renaming a function while preserving its body should be expressible as:

```diff
@@ -10,7 @@ rule=rename_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }
```

instead of repeating the entire function body.

### 2.2 Git familiarity

RSmPL-Git deliberately resembles a Git patch:

- file sections use `diff --git`;
- paths use `---` and `+++`;
- ranges use `@@ -old,+new @@`;
- ordinary hunks remain ordinary Git hunks;
- semantic hunks use `-`, `+`, and context prefixes.

The extension should feel like “Git patch plus semantic hunks,” not a totally separate refactoring language.

### 2.3 Mandatory locality

Every RSmPL semantic transformation must be scoped to a file and line range.

This is achieved by reusing the surrounding Git file section and hunk header range:

```diff
diff --git src/lib.rs
@@ -40,8 @@ rule=rename_calls
# allow=many
# expr E
- foo(E)
+ bar(E)
```

The engine searches only inside `src/lib.rs` lines `40..47` on the old side.

### 2.4 Explicit syntax

RSmPL must not silently generalize Rust syntax.

This pattern:

```rust
fn foo(...) {
    ...
}
```

matches plain functions named `foo`, such as:

```rust
fn foo() {}
```

and:

```rust
fn foo(x: i32) -> i32 {
    x + 1
}
```

It does **not** match:

```rust
pub async unsafe fn foo<T>(x: T)
where
    T: Clone,
{
    x
}
```

because the pattern did not explicitly include `pub`, `async`, `unsafe`, generic parameters, or a `where` clause.

### 2.5 Semantic verification

The engine should verify transformations structurally and, when requested, semantically.

For example, a semantic rename should be able to distinguish:

```rust
crate::foo()
```

from another `foo()` that resolves to a different symbol.

### 2.6 Failure over guessing

If a rule cannot be applied exactly as specified, the engine should fail with a clear diagnostic rather than guessing.

---

## 3. Relationship to Git Patches

RSmPL-Git is intentionally close to Git unified-diff syntax, but an RSmPL hunk is not necessarily accepted by vanilla `git apply`.

### 3.1 Ordinary hunks

Ordinary hunks are valid Git unified-diff hunks and may be passed through to `git apply`.

Example:

```diff
@@ -1,4 +1,5 @@
 mod old;
+mod new;

 fn main() {}
 }
```

### 3.2 Semantic hunks

Semantic hunks use Git-like hunk headers and diff line prefixes, but include unprefixed directive lines immediately after the hunk header:

```diff
@@ -40,8 @@ rule=rename_calls
# allow=many
# expr E
- foo(E)
+ bar(E)
```

These unprefixed `#` directive lines are not valid ordinary unified-diff body lines. They are an RSmPL-Git extension.

Therefore:

- A file containing only ordinary hunks should be usable as a normal Git patch.
- A file containing RSmPL hunks requires an RSmPL-aware engine.
- After application, the engine should be able to emit a concrete normal Git diff.

### 3.3 Why keep Git-like syntax?

Git-like syntax gives the LLM and human reviewer:

- the target file;
- a target line range;
- a familiar patch shape;
- compact reviewable intent;
- mixed semantic and textual edits in one file.

---

## 4. Relationship to Coccinelle SmPL

RSmPL is inspired by Coccinelle SmPL but is not a byte-for-byte clone.

The retained ideas are:

- source-code-like matching patterns;
- metavariable declarations;
- `...` for irrelevant syntax;
- patch-style `-` and `+` transformations;
- semantic matching rather than pure text substitution.

The Rust-specific differences are:

- Rust syntax and Rust AST are the target;
- every rule is scoped by Git file/range information;
- hunk metadata is encoded as compact `#` directives;
- the patch file is organized like a Git patch;
- semantic rename can use Rust name resolution;
- Rust macros require explicit handling modes.

---

## 5. Core Concepts

### 5.1 RSmPL-Git patch

An entire patch document containing one or more Git-style file sections.

### 5.2 File section

A section beginning with:

```diff
diff --git a/path b/path
```

and followed by file metadata such as:

```diff
--- a/path
+++ b/path
```

### 5.3 Hunk

A range-based edit beginning with:

<!-- rsmpl:invalid RSMPL_PARSE_CONFLICT -->
```diff
@@ -old_start,old_len +new_start,new_len @@
```

### 5.4 Ordinary hunk

A hunk with no RSmPL directive block. It is interpreted as a normal Git unified-diff hunk.

### 5.5 RSmPL hunk

A hunk with an RSmPL directive block immediately after the hunk header. It is interpreted as a Rust semantic rewrite.

### 5.6 Directive block

One or more unprefixed lines beginning with `#`, placed immediately after the hunk header.

Example:

```diff
# lang=rust
# allow=many
# expr E
```

### 5.7 Semantic range

The old-side hunk range used as the AST search extent.

For RSmPL hunks, semantic ranges are interpreted in the **original-source coordinate system** of the patch base. They are not reinterpreted after earlier hunks have changed the file.

### 5.8 Original-source coordinate system

The coordinate system defined by the target file contents before any hunk in the patch file is applied.

All RSmPL `-old_start,old_len` ranges refer to this original source file. This is a deliberate divergence from an immediate sequential patch-application model and is required to make LLM-authored semantic patches stable under previous rewrites, moves, and formatter output.

### 5.9 Pattern body

The `-`, `+`, and context lines in an RSmPL hunk, parsed as Rust semantic patterns rather than literal source lines.

---

## 6. Patch File Structure

A patch file consists of zero or more file sections.

```ebnf
patch           ::= file_section+

file_section    ::= git_file_header file_body

git_file_header ::= "diff --git " path (" " path)? newline
                    git_metadata*

file_body       ::= hunk*

hunk            ::= ordinary_hunk | rsmpl_hunk
```

A single path (`diff --git src/lib.rs`) is equivalent to the duplicated
two-path form (`diff --git a/src/lib.rs b/src/lib.rs`); an optional
`a/`/`b/` prefix is stripped either way. The two-path form stays for
compatibility with Git tooling. The two stripped paths are usually
identical; differing real paths are a file rename section (T377):
hunks cite old-side coordinates and apply against the old file's
content, and the section lowers back to `rename from`/`rename to`
(+ `---`/`+++` and hunks when the content differs; headers only for a
pure rename, exactly git's shape). RSmPL hunks may appear in rename
sections and rewrite old-side content. `/dev/null` halves stay the create/delete spelling
(`diff --git /dev/null b/new`, `diff --git a/old /dev/null`).
Paths may be quoted; unquoted paths may contain spaces (real git never
quotes spaces) — the header splits at the `b/`-led part whose stripped
sides are equal (bare-git parity, T397 — the old first-`b/` rule
mis-targeted paths holding an interior `b/…` segment), else the first
`b/`-led part (differing halves are renames), or at a `/dev/null`
half, else it fails to parse (T379). `rename from`/`rename to` lines
override the halves (bare git resolves spaced renames from them, T397).
Emitted headers C-quote slots holding control chars, `"` or `\`
(spaces stay raw, T398). Quoted paths decode
git's C-style escapes (default `core.quotepath` quotes non-ASCII names,
so `"caf\303\251.txt"` resolves to `café.txt`, T389).
A two-part remainder that wears neither git's `a/`/`b/` uniform nor a
`/dev/null` half is one single path with a space (SPEC 6 single-path form,
T381) — never a rename of the first word into the second.
An identical duplicated bare remainder (`diff --git A A`) is the single path `A` — quote (`diff --git "A A"`) to name a file literally containing the duplication.
Repeated sections for one path are allowed and share original coordinates (Section 10): every `-old,len` cites the pre-patch file, never rebased by earlier sections or hunks.
Line endings: a trailing `\r` on a hunk body line (` `/`-`/`+`) is file
content and survives byte-exact (CRLF files round-trip unchanged,
T385); structural lines (`diff --git` headers, hunk headers,
directives, metadata) tolerate one trailing `\r`, so CRLF-authored
patch files parse as before.

Example:

```diff
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,7 @@ rule=rename_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }

@@ -1,4 +1,5 @@
 mod old;
+mod new;

 fn main() {}
 }
```

The file contains:

1. one RSmPL hunk;
2. one ordinary Git hunk.

The `---` / `+++` / `index` lines above are optional file metadata
(`git_metadata*` in the grammar); the parser only requires the
`diff --git` line before the hunks. An empty patch (zero file sections) is
rejected with `RSMPL_PARSE_CONFLICT` (verbatim fence in Section 25.2): an empty patch
input (stdin, `--file`, or `--patch`) is almost certainly an authoring typo,
and silent success would violate Section 2.6. (A non-empty patch
whose hunks lower to no changes still succeeds with an empty
diff.) The canonical agent recipe is
therefore the 1-line header plus an elided new side:

```diff
diff --git src/lib.rs
@@ -10,7 @@
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }
```

No `---` / `+++` / `index` lines, no `rule=` tag (diagnostics then
show `<unnamed>`; add `rule=name` when a patch has several hunks).
The single `# lang=rust` line is the RSmPL marker here: a hunk with
no `#` lines at all would be an ordinary Git hunk (Section 7.2), so
a bare rename keeps the marker while hunks that already carry other
directives (such as `# allow=many` / `# expr E`) omit it.
For RSmPL hunks the `+new,len` side is ignored (Section 10.4), so it
may be elided (`@@ -10,7 @@` defaults to the old side); copying the
old side verbatim (`@@ -10,7 +10,7 @@`) stays compatible.

---

## 7. RSmPL Hunk Detection

### 7.1 Detection rule

A hunk is an RSmPL hunk if and only if:

1. it has a normal unified-diff hunk header; and
2. the header carries `rule=`, or immediately after the hunk header there
   is at least one unprefixed directive line beginning with `#`.

A `rule=`-only hunk carries no `#` directives, so an ordinary-shaped text edit tagged with `rule=` is compiled as a pattern and fails with `RSMPL_PATTERN_CONFLICT`; that failure carries the suggestion "remove `rule=` for plain text hunks (or add `#` directives for a semantic pattern)".

The tool is Rust-only, so the language defaults to Rust. A
`# lang=rust` directive may be included for explicitness but is never
required; `# lang=<other>` is rejected.

Because detection needs `rule=` or at least one `#` line, a hunk whose
shape requires no other directive (such as a bare rename) writes only
the terse form plus `rule=`:

```diff
@@ -10,7 @@ rule=rename_foo
- fn foo
+ fn bar
```

while a hunk that already carries other directives omits it:

```diff
@@ -40,8 @@ rule=rename_calls
# allow=many
# expr E
- foo(E)
+ bar(E)
```

Both are RSmPL hunks.

### 7.2 Ordinary hunk detection

A hunk is ordinary if the first line after the hunk header is a normal diff body line starting with one of:

```text
' '
'+'
'-'
'\\'
```

Example:

```diff
@@ -1,4 +1,5 @@
 mod old;
+mod new;

 fn main() {}
 }
```

This is an ordinary Git hunk.

### 7.3 Directive line position

Directive lines must appear immediately after the hunk header and before any hunk body line.

Valid:

```diff
@@ -40,8 @@ rule=rename_calls
# expr E
- foo(E)
+ bar(E)
```

Invalid:

<!-- rsmpl:invalid RSMPL_PATTERN_CONFLICT -->
```diff
@@ -40,8 @@ rule=rename_calls
- foo(E)
# lang=rust
+ bar(E)
```

Engine-verbatim misplaced-directive rejection (exit 2; reason + set
suggestion; text omits the null `rule:`/`file:`/`range:` lines, JSON
carries explicit nulls):

```text
RSMPL_PATTERN_CONFLICT
reason: directive line must appear immediately after hunk header, before body lines
suggestion: move the `#` line above the first `-`/`+`/` ` line
```

The canonical fence lives in Section 25.3, which contrasts it with the
empty-body rejection (rule-only envelope).

### 7.4 Directive lines are not source lines

Directive lines are not part of the source file and are not counted against old or new hunk line counts.

In this hunk:

```diff
@@ -40,8 @@ rule=rename_calls
# allow=many
# expr E
- foo(E)
+ bar(E)
```

the two `#` lines are metadata. They do not represent Rust comments, removed lines, added lines, or context lines.

### 7.5 No ambiguity with Rust comments or attributes

A Rust source comment inside a diff hunk has a diff prefix:

```diff
 // this is a Rust source comment
```

An added Rust attribute has a diff prefix:

```diff
+#[derive(Debug)]
```

A removed Rust attribute has a diff prefix:

```diff
-#[derive(Debug)]
```

An RSmPL directive has no diff prefix:

```diff
# lang=rust
```

Therefore directives are unambiguous.

---

## 8. RSmPL Hunk Header Syntax

An RSmPL hunk uses a standard-looking unified-diff hunk header, with
the new-side range optionally elided:

<!-- rsmpl:invalid RSMPL_PARSE_CONFLICT -->
```diff
@@ -old_start,old_len +new_start,new_len @@ rule=name
```

<!-- rsmpl:invalid RSMPL_PARSE_CONFLICT -->
```diff
@@ -old_start,old_len @@ rule=name
```

The elided form defaults the new side to the old side and is
RSmPL-only: ordinary hunks (no `#` directives) must keep both ranges,
because Git semantics need them. An elided new side on an ordinary
hunk is rejected with `RSMPL_PARSE_CONFLICT`.

Engine-verbatim (exit 2; the reason echoes the offending header line;
text omits the null `rule:`/`file:`/`range:` lines, JSON carries
explicit nulls):

```text
RSMPL_PARSE_CONFLICT
reason: ordinary hunk requires both -old,len +new,len ranges: @@ -1,1 @@
suggestion: write both ranges (e.g. `@@ -1,2 +1,2 @@`) or add a `#` directive line to make it an RSmPL hunk
```

The canonical fence lives in Section 25.2.

Example:

```diff
@@ -10,7 @@ rule=rename_foo
- fn foo
+ fn bar
```

### 8.1 Header fields

The hunk header contains:

| Field | Meaning |
|---|---|
| `old_start` | starting line in the pre-application file |
| `old_len` | size of the old-side semantic extent |
| `new_start` | expected or intended starting line in the post-application file |
| `new_len` | intended new-side extent |
| trailing text | machine-readable tags and/or human section text |

### 8.2 `rule` tag

The trailing hunk header should include:

```text
rule=<identifier>
```

Example:

```diff
@@ -80,20 @@ rule=rename_calls
- foo(E)
+ bar(E)
```

The rule name is used for diagnostics, captures, dependency tracking, and review display. It is RECOMMENDED for multi-hunk patches but OPTIONAL for single-hunk patches; a hunk without `rule=` is valid and diagnostics show `<unnamed>` for it. Names are per-hunk labels and need not be unique: reports list one entry per hunk in document order, so same-named hunks are distinguished by file and range.

### 8.3 Header tag syntax

Header tags use whitespace-separated key-value pairs:

```text
key=value
```

Recommended header tags:

```text
rule=<identifier>
```

No other header tag is supported in v0.1: any `k=v` token other than
`rule=` (including former `id=`/`depends=`/`extra` spellings, which the
engine never read) is rejected with `RSMPL_PATTERN_CONFLICT` — accepting
dead tags would be a silent no-op against Section 2.6. Bare words without
`=` are human section text and stay ignored.

Engine-verbatim (exit 2; `bogus=1` is the offending token, backticked):

```text
RSMPL_PATTERN_CONFLICT
old_range: line 2
reason: unknown header tag `bogus=1` (only `rule=` is supported in v0.1; remove it, see SPEC 8.3)
```

The failure carries no `rule:`/`file:`/`suggestion:` lines (null fields are
omitted from text output; the JSON envelope carries explicit `rule: null` /
`file: null` / `range: "line 2"` / `suggestion: null` — `line 2` is the
hunk-header line number, not a `start..start+len` range).

Most RSmPL-specific configuration should be placed in directive lines, not the hunk header.

Preferred:

```diff
@@ -40,8 @@ rule=rename_calls
# allow=many
# expr E
- foo(E)
+ bar(E)
```

Rejected (unknown header tags fail fast, Section 8.3):

<!-- rsmpl:invalid RSMPL_PATTERN_CONFLICT -->
```diff
@@ -40,8 +40,8 @@ rule=rename_calls lang=rust allow=many expr=E
```

The hunk header should stay close to normal Git style.

Malformed hunk headers fail fast with `RSMPL_PARSE_CONFLICT` (exit 2).
All three share one envelope: `old_range: line N` (the hunk-header line
number, not a `start..start+len` range; null `rule:`/`file:`/
`suggestion:` lines omitted from text, explicit nulls in JSON).
Engine-verbatim (`{line}` / `{range}` echo the offending spelling):

```text
RSMPL_PARSE_CONFLICT
old_range: line 2
reason: invalid hunk header (missing closing @@): @@ -1,1 rule=r
```

```text
RSMPL_PARSE_CONFLICT
old_range: line 2
reason: invalid hunk header ranges: @@ -1,1 +1,1 +1,1 @@ rule=r
```

```text
RSMPL_PARSE_CONFLICT
old_range: line 2
reason: invalid hunk range: -foo
```

The bare `invalid hunk header: {line}` arm (no leading `@@`) is
defensive-only: the header parser only runs on `@@`-led lines, so the
CLI never emits it (T287/T290/T291 precedent — documented, not pinned).
The canonical fences live in Section 25.2.

---

## 9. RSmPL Directive Lines

RSmPL directive lines are compact metadata lines placed immediately after the hunk header.

### 9.1 General syntax

```text
# key=value
# flag
# metavariable-kind NAME
# metavariable-kind NAME = constraint
```

Examples:

```text
# lang=rust
# allow=many
# optional
# scope=contained
# attrs=preserve
# expr E
# item I
# symbol Old=crate::foo
```

### 9.2 Language directive (optional)

RSmPL hunks default to Rust. A hunk may include:

```text
# lang=rust
```

for explicitness, but it is not required. Any `# lang=` value other
than `rust` is rejected. Because hunk detection (Section 7.1) needs
`rule=` or at least one `#` directive line, a hunk whose shape requires
no other directive (such as a bare rename) writes the terse form plus
`rule=` with no directive lines at all;
hunks that already carry other directives omit it.

### 9.3 Core directives

| Directive | Meaning |
|---|---|
| `# lang=rust` | Accepted for explicitness; optional — hunks default to Rust and are detected by `rule=` or any `#` line (Section 7.1). `# lang=<other>` is rejected. |
| `# allow=one` | Exactly one match is expected. Default. |
| `# allow=many` | Multiple matches may be rewritten. |
| `# optional` | Zero matches are allowed. |
| `# optional=true` | Equivalent to `# optional`. |
| `# optional=false` | Zero matches are an error. Default. |
| `# repeat` | Re-apply the same rewrite inside captured bindings, bottom-up: one hunk folds nested matches (Section 17.8). Call/method-call and general-expression shapes in v0.1; `...` bodies are opaque. |
| `# scope=contained` | Matched AST node must be fully inside the old range. Default. |
| `# scope=anchor` | First concrete non-ellipsis token must be inside the old range. |
| `# scope=overlap` | Matched AST node may overlap the old range. |
| `# scope=exact` | Matched AST node span must exactly match the old range after trivia normalization. |
| `# scope=file` | Search the whole file; the `@@` old range is a dummy (canonical `@@ -1,1 @@`) and range-bounds validation is skipped. Insertion anchors ignore scope. |
| `# sweep` | Exactly `# allow=many` + `# scope=file` (fused sweep header). Ranged multi-match keeps spelling `# allow=many` without `# scope=file`. |
| `# bind=Name` | Capture exactly one rewritten or removed AST object under a cross-hunk name — or, combined with `# allow=many` on an item cut, every matched item in source order (ordered collection, Section 20.3). |
| `# use=Name` | Use a previously captured scalar object. |
| `# attrs=preserve` | Ignore unwritten attached attributes for match eligibility and preserve them across rewrite, deletion capture, and movement. Default. Explicitly written attributes constrain the match (exact match, other sites skipped); the plus side must mirror the minus side. |
| `# attrs=absent` | Require no attached attributes or doc comments. |
| `# attrs=explicit` | Only attributes written in the pattern participate; unwritten attached attributes cause the match to fail. |
| `# attrs=strip` | Match like `preserve`, but do not copy attributes to the replacement when stripping is valid. |
| `# mode=ast` | Parse and match using AST. Default. |
| `# mode=tokens` | Match using token trees or token stream. (> v0.1 note: only supported for macro token-tree patterns; otherwise rejected.) |
| `# macro=invocation` | Treat macro invocations as AST nodes. Default. |
| `# macro=token_tree` | Allow token-tree matching inside macro invocation bodies. |
| `# macro=expanded` | Match macro-expanded output when build integration is available. |
| `# format=preserve` | Preserve original formatting where possible. Default recommendation. |
| `# format=rustfmt` | Rejected in v0.1 (run `cargo fmt` after apply, Section 22.2). |
| `# format=canonical` | Rejected in v0.1 (run `cargo fmt` after apply, Section 22.3). |
| `# returns=none` | Minus-side requires no explicit return type (function items only, Section 14.2). |
| `# root=item/expr/stmt/type` | Force the parse context when the shape is ambiguous (Section 13.4). |
| `# fallback=syntax` | Allow syntactic match when `# symbol` cannot resolve (required in v0.1, Section 19); also the WI-6a follow skip switch (Section 14.8). |
| `# follow=uses` | Rewrite same-file use sites alongside a fn/item rename (WI-6a, Section 14.8; legal only on fn/item rename hunks). |
| `# symbol Name=path` | Bind a semantic symbol to a fully-qualified path (Sections 9.5, 19). |
| `# metavars=sigiled/bare` | Sigiled mode requires `$Name` in bodies; bare is the token-efficient default (Section 15.5). |

### 9.4 Metavariable directives

Metavariables are declared in directive lines:

```text
# expr E
# type T
# item I
```

A metavariable declaration has:

```text
# <kind> <name>
```

The full implemented kind list lives in Section 15.1 (`symbol`, `expr`, `stmt`, `item`, `type`, `pat`, `generic_params`, `where_clause`, `visibility`, `qualifier`, `qualifier_list`, `block`, `macro`, `token_tree`; `identifier`/`path`/`lifetime`/`field`/`attributes` are rejected).

Multiple metavariables of the same kind may be declared in separate lines:

```text
# expr Receiver
# expr Arg
```

or in a comma-separated form, which declares each name under the same
kind exactly as separate lines do:

```text
# expr Receiver, Arg
```

Whitespace around the commas is ignored and a trailing comma is
harmless. The separate-line form is canonical. Write `# expr E, F`,
not `# expr E F` (a space-separated list is a pattern error), and
note `# symbol` keeps the single-`Name=path` form (Section 9.5) with
no comma form.

A space-separated name list is a pattern error, not two declarations:
`# expr E F` fails fast with `RSMPL_PATTERN_CONFLICT` (exit 2;
parse-time bare envelope — text carries the bare `reason:` line only,
JSON carries explicit `rule: null` / `file: null` / `range: null` /
`suggestion: null`). Engine-verbatim (`{name}` is the trimmed
comma-segment containing the space, `{line}` the full offending `#`
line including any `= constraint` tail):

```text
RSMPL_PATTERN_CONFLICT
reason: invalid metavariable name `E F` in: # expr E F
```

The gate fires per comma-segment before charset validation, so it is
distinct from the Section 9.6 `invalid name` gates (which reject one
bad identifier — `` `1bad` `` at the start-char gate, `` `foo-bar` ``
at the alphanumeric gate — after the whitespace split). The fix is the
comma form above (`# expr E, F`).

Re-declaring one name under one kind is allowed and idempotent
(`# expr E` twice binds once). Re-declaring one name under two
different kinds fails fast with `RSMPL_PATTERN_CONFLICT` (exit 2;
rule + file lines; text omits the null range/suggestion lines, JSON
carries explicit `range: null` / `suggestion: null`). The check runs
in declaration order across both directive families, so `# expr E`
then `# symbol E=...` fails the same way as the reverse order (the
second declaration loses — there is no shadowing); a repeated
`# symbol Old=...` under the same name keeps the `symbol` kind and the
second path wins. `{name}` echoes the offending name, `(prev vs new)`
echoes the kind spellings in declaration order. Engine-verbatim:

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: src/lib.rs
reason: metavariable `E` declared twice with different kinds (expr vs type)
```

### 9.5 Symbol directives

A semantic symbol binding is declared with:

```text
# symbol Name=fully::qualified::path
```

Example:

```diff
@@ -80,20 @@ rule=rename_symbol
# allow=many
# fallback=syntax
# symbol Old=crate::old_api
# symbol New=crate::new_api
- Old(...)
+ New(...)
```

The engine must verify that `Old` resolves to `crate::old_api` before rewriting it. Resolution is crate-local from `syn` alone (Section 19.3): seeded `crate::` paths must name a definition in the indexed crate (file/inline `mod` nesting, `self`/`super`, one-hop `pub use` re-exports with cycle cut; glob imports are opaque). A bare `# symbol` hunk whose seed resolves rewrites only the sites that resolve to the same definition (a same-text vendored copy stays untouched); without it the hunk fails with `RSMPL_SYMBOL_CONFLICT`. `# fallback=syntax` keeps the exact full-path text match with no resolution.

### 9.6 Directive parsing

Directive blocks end at the first hunk body line.

A hunk body line begins with one of:

```text
space
+
-
\
```

Everything before that, if it is an unprefixed `#` line, is a directive.

Unknown directives are rejected with `RSMPL_PATTERN_CONFLICT` (no
forward-compat silent ignore, Section 2.6): an agent that authors `# foo`
or `# foo=bar` gets `unknown directive` plus the Section 9.3 pointer
instead of a silently ignored no-op. `# edition=*` and `# format_scope=*`
are rejected as unsupported in v0.1 (edition is the toolchain default;
only `# format=preserve` is supported).
Both fail parse-time (exit 2; bare + suggestion envelope — text carries
the `reason:` + `suggestion:` lines only, null rule/file/range omitted;
JSON carries explicit `rule: null` / `file: null` / `range: null` plus the
string suggestion). Engine-verbatim (`{v}` echoes the offending value,
`{line}` the full `#` line):
```text
RSMPL_PATTERN_CONFLICT
reason: unsupported directive `# edition=2021` (edition is the toolchain default in v0.1): # edition=2021
suggestion: remove the directive — edition is the toolchain default in v0.1
```
```text
RSMPL_PATTERN_CONFLICT
reason: unsupported directive `# format_scope=file` (only `# format=preserve` is supported in v0.1): # format_scope=file
suggestion: remove the directive — only `# format=preserve` is supported
```
The edition arm fires on any `# edition=<value>` spelling (the toolchain
default applies); the format_scope arm on any `# format_scope=<value>`
spelling (only `# format=preserve` exists in v0.1). Unlike the
suggestion-less canonical unknown-directive fence, both carry a set
suggestion (remove the directive); the typo near-miss is the only other
`reason:` in this section with one.
The three unknown-directive shapes below all fail parse-time (exit 2, bare envelope — text carries the bare `reason:` line only, JSON carries explicit `rule: null` / `file: null` / `range: null`).
```text
unknown directive `#foo=bar`: # foo=bar
unknown directive `#alow=many` (did you mean `#allow=...`?): # alow=many
unknown directive `#foo`: # foo
```
The first carries suggestion `check SPEC 9.3 for the supported directive list`; the typo near-miss carries `use `#allow=...`` (one-edit distance over `KNOWN_KV_KEYS`, `alow` → `allow` probed); the bare/multi-token form echoes only the first token (both `# foo` and `# foo bar` echo `` `#foo` ``) with the generic 9.3 pointer. The `invalid directive: {line}` arm is defensive-only: the parser only collects `#`-led lines, so no CLI input reaches it (T287/T290/T291 precedent, documented not pinned).

Bad *values* for known directives fail the same way (`RSMPL_PATTERN_CONFLICT`,
exit 2, bare `reason:` line — null rule/file/range omitted from text
(explicit nulls in JSON); the text `suggestion:` line appears only when the
reason sets one, which only the `repeat` entry below does). Engine-verbatim
the offending spelling, echoed after the colon):

```text
unsupported language `bogus` (only `rust` supported): # lang=bogus
invalid scope `bogus`: # scope=bogus
invalid allow value `bogus` (want one|many): # allow=bogus
invalid optional value `bogus`: # optional=bogus
invalid attrs value `bogus`: # attrs=bogus
invalid mode `bogus`: # mode=bogus
invalid macro mode `bogus`: # macro=bogus
invalid format `bogus`: # format=bogus
invalid fallback `bogus`: # fallback=bogus
invalid returns `bogus` (only `none` supported): # returns=bogus
invalid metavars `bogus`: # metavars=bogus
invalid repeat value `bogus` (write bare `# repeat`): # repeat=bogus
invalid symbol directive (want `# symbol Name=path`): # symbol JustName
metavariable directive missing name: # expr
invalid name `1bad` (must start with letter or _)
invalid name `foo-bar` (only alphanumeric + _ allowed)
reserved name `ELLIPSIS_FOO` (`ELLIPSIS*` names the `...` marker; rename the metavariable, e.g. `E`)
symbol path must not be empty: # symbol Old=
empty metavariable/capture name
empty directive: #
```

The last six are malformed declarations or empty lines, not values: a `=`-less `# symbol`
line, a nameless `# expr`-family line, two bad-name spellings, an empty path or
capture name, and a lone `#`. Engine-verbatim (all exit 2; all parse-time, before any
`rule:`/`file:` is attached — text carries the bare `reason:` line only;
JSON carries explicit `rule: null` / `file: null` / `range: null` /
`suggestion: null`):

```text
RSMPL_PATTERN_CONFLICT
reason: metavariable directive missing name: # expr
```

```text
RSMPL_PATTERN_CONFLICT
reason: invalid name `1bad` (must start with letter or _)
```

```text
RSMPL_PATTERN_CONFLICT
reason: invalid name `foo-bar` (only alphanumeric + _ allowed)
```

```text
RSMPL_PATTERN_CONFLICT
reason: symbol path must not be empty: # symbol Old=
```

```text
RSMPL_PATTERN_CONFLICT
reason: empty metavariable/capture name
```

```text
RSMPL_PATTERN_CONFLICT
reason: empty directive: #
```

`{line}` echoes the offending `#` directive line (`# expr` bare, `# symbol
Old=` with its empty path, lone `#` verbatim); backticked names echo the
offending spelling. The first fires when the directive carries no name at
all; the next two split name validation — a leading digit fails the
start-char gate (`` `1bad` ``), other charset violations (`` `foo-bar` ``)
fail the alphanumeric gate; the fourth fires when a `# symbol Name=` line
carries no path; the fifth only for empty `# bind=`/`# use=` names; the
sixth fires on a lone `#` line (trailing whitespace ignored). Note the
engine's own wording —
`allow`/`attrs`/`optional`/`repeat` say "value", the rest don't; only `allow`
carries `(want one|many)`, only `returns` carries `(only `none` supported)`,
only `repeat` carries `(write bare `# repeat`)` plus the echoed offending
`# repeat=<value>` spelling — recorded verbatim, not normalized. All value
fences above carry a null suggestion (omitted from text, explicit null in
JSON) except `invalid repeat value`, whose suggestion is `write bare `# repeat`
with no value` (text carries the `suggestion:` line, JSON carries the string).

---

## 10. Line Range Semantics

### 10.1 Source of file and range

RSmPL-Git does not use separate `file:` and `lines:` fields.

The file is determined by the surrounding Git file section:

```diff
diff --git src/lib.rs
```

The range is determined by the hunk header:

```diff
@@ -40,8 @@ rule=rename_calls
- foo(E)
+ bar(E)
```

This gives the old-side semantic search range:

```text
src/lib.rs lines 40 through 47
```

### 10.2 Original-source coordinate system

For every RSmPL hunk, the old-side range:

```text
-old_start,old_len
```

is interpreted against the **original file contents before any hunk in the patch file is applied**. The sole exception is `# scope=file` (Section 17.3), where the range is a dummy and the whole file is searched.

This rule is mandatory.

It means that an LLM may cite the line numbers it saw in the original source file for every semantic hunk, even if earlier hunks in the same patch:

- insert lines;
- delete lines;
- move items;
- rewrite an item into a different number of lines;
- run `rustfmt` on an affected node or range;
- otherwise cause line-number drift in the materialized output.

RSmPL engines must not require the author to predict accumulated line offsets produced by previous semantic rewrites.

### 10.3 Old-side range

For RSmPL hunks, the old-side range:

```text
-old_start,old_len
```

defines the region in the original source file where AST matching is permitted. With `# scope=file` the range is a dummy (canonical `@@ -1,1 @@`); the engine searches the whole file and skips range-bounds validation for that hunk.

Example:

```diff
@@ -40,8 @@ rule=rename_calls
- foo(E)
+ bar(E)
```

means the engine may search only original old lines:

```text
40, 41, 42, 43, 44, 45, 46, 47
```

The engine should retain original AST span mappings while planning the patch. After matching, the engine may rewrite, move, or format the corresponding AST nodes, but later RSmPL hunks must still resolve their old-side ranges against the original source coordinate system.

### 10.4 New-side range

For RSmPL hunks, the new-side range:

```text
+new_start,new_len
```

is a review-oriented declaration of the intended output extent. It is not used to define the search range for later RSmPL hunks.

Because RSmPL bodies may contain ellipses, metavariables, captures, moved AST nodes, and formatter output, the new-side line count is an **extent declaration**, not a literal promise that the resulting output will contain exactly that number of lines.

Implementations should tolerate new-side line-count drift for RSmPL hunks and compute the final concrete line counts when lowering the RSmPL-Git patch to an ordinary Git diff. The new-side values are not consulted for RSmPL matching at all, so an agent that wants zero decision cost may elide the new side (`@@ -10,7 @@`, which defaults to the old side) or copy the old side verbatim (e.g. `@@ -10,7 +10,7 @@`, kept for compatibility).

For example:

```diff
@@ -20,12 @@ rule=cut_helper
# item I
# bind=Helper
- I
```

contains only one removed pattern line, but the old-side extent is 12 original source lines.

### 10.5 Zero-length ranges and insertion anchors

Zero-length old-side ranges are allowed for insertions.

Example:

```diff
@@ -5,0 @@ rule=paste_helper
# use=Helper
+ Helper
```

The insertion anchor is line 5 in the original source file. The inserted code length is determined by the captured AST object, the target syntactic position, and the formatter.

For insertions, `-old_start,0` is the normative anchor. `+new_start,new_len` remains review metadata and must not be used to rebase later semantic hunk ranges.

> **v0.1 implementation note:** the anchor `-N,0` inserts *after* line `N` (at the start of line `N+1`, or EOF). `-0,0` inserts at the start of the file. `-len-1,0` (one past the last line) is the EOF spelling and is equivalent to `-len,0`; anything further fails `RSMPL_RANGE_CONFLICT`. Deleted-item cut ranges additionally absorb one adjacent blank separator line (following blank preferred, else preceding at EOF) so moves do not leave stray blank lines; the captured text itself excludes the separator. Same-anchor multi-pastes restore one blank line between items on insertion (Section 20.2).

### 10.6 Interaction with ordinary Git hunk coordinates

Ordinary Git hunks retain normal unified-diff semantics.

However, when an RSmPL-aware engine processes a mixed patch, it should plan all edits for a file against the original source coordinate system before materializing the final file. This gives both ordinary and semantic hunks a stable common base for conflict detection.

If an ordinary hunk and an RSmPL hunk edit overlapping original source ranges, the engine must either:

1. prove that the edits are compatible and merge them deterministically; or
2. reject the patch with an overlap conflict.

Engine-verbatim overlap rejection (exit 2; any overlapping pair -- semantic-semantic, ordinary-semantic, or an insertion strictly inside a deleted/replaced range):

```text
GIT_HUNK_CONFLICT
file: src/lib.rs
reason: overlapping edits cannot be merged
suggestion: apply the overlapping rewrites in separate runs, or make the ranges disjoint
```

The failure carries `file:` but no `rule:`/`old_range:` lines (null fields are omitted from text output; the JSON envelope carries explicit `rule: null` / `range: null` with `file` set and the same `reason`/`suggestion` strings). Compatible (disjoint) edits merge normally per option 1 above. The canonical fence lives in Section 25.7, which contrasts this shape with the ordinary-hunk content conflict.

The engine must not silently reinterpret an RSmPL old-side range using accumulated offsets from previous ordinary or semantic hunks.

### 10.7 Rationale: avoiding line-number drift for LLMs

RSmPL-Git targets LLM-generated patches. LLMs commonly know the original source lines they inspected, but they cannot reliably predict how many lines a semantic rewrite, moved item, pretty-printer, or `rustfmt` pass will produce.

Therefore, semantic hunk ranges are stable references into the original file, not sequential offsets into a changing intermediate file.

The correct processing model is:

```text
parse original file
record original AST spans
match all RSmPL hunks against original spans
build an edit plan
resolve dependencies and conflicts
materialize final file
emit concrete Git diff if requested
```

### 10.8 Drift fallback (stale ranges)

A ranged hunk authored against shifted line numbers finds zero matches
in its cited range. When that happens the engine retries once with file
scope under one of two tiers. Anchored tier: the hunk's body context
lines anchor the single file-wide candidate — leading ` ` lines (before
the first `-`/`+` line) must occur trimmed-equal in order in the 10 lines
before it, trailing ` ` lines (after the last `-`/`+` line) in order in
the 10 lines after it. Whitespace-only and `...`-only lines carry no
disambiguation and are skipped. Pattern-anchored tier (terse hunks): a
hunk with no body context lines drifts on the pattern's uniqueness plus
disjointness alone. Both tiers emit identical `drift` evidence (`{rule,
file, cited: [s,e], actual: [a,b]}`, Section D.2; one `drift:` stderr line
in text mode) only when the cited range shares no line with the drifted
item's enclosing top-level item: an overlapping range is a scope/filter
statement about that item (Section 17.3), not line drift. Anchors present
must agree with the unique candidate — disagreement keeps the original
conflict. Zero or several candidates, or an overlapping range, keep the
original `RSMPL_MATCH_CONFLICT` and apply nothing. `# scope=file` and
sweep hunks never drift (they make no locality claim), nor do insertions
(anchor-positioned already), `# optional` hunks (zero is success), ranged
`# allow=many` (the exactly-one gate contradicts fan-out — use `# scope=file`
for mobile sweeps), or header-context hunks (`impl` / trait contexts,
member renames: the header is already a precise file-wide selector, so a
range/header disagreement is an authoring error the Section 13.4 pins keep
loud). Emitted edits still cite original coordinates and the Section 24.6
postconditions still gate.

---

## 11. Ordinary Git Hunks

A hunk without a directive block is ordinary.

Example:

```diff
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 mod old;
+mod new;

 fn main() {}
```

An RSmPL-Git engine applies ordinary hunks using standard unified-diff semantics or delegates them to a Git-compatible patch applier. Header counts must agree with the body (validated at parse, below): old-side lines (` ` + `-`) equal `-old,len` and new-side lines (` ` + `+`) equal `+new,len`; `\ No newline` markers are neither side. The engine positions by `old_start` and verifies every context (` `) and removal (`-`) line byte-for-byte against the original file. `old_len` still bounds the file (`old_start-1+old_len` must fit, else `RSMPL_RANGE_CONFLICT`); a content mismatch (wrong `old_start` or wrong line text) fails with `GIT_HUNK_CONFLICT`. RSmPL hunks are exempt from count validation: counts are search ranges there and bodies are patterns (Section 10.4).

Ordinary hunks may appear before, between, or after RSmPL hunks.
Patch prose between hunks or sections (bare lines carrying no ` /+/-/\\` prefix) is skipped and never affects the lowering; a line starting with `@@` is always a hunk-header attempt and fails loud when malformed, while ` /+/-/\\`-leading lines are hunk body lines under standard unified-diff semantics, never prose.
Inside an ordinary hunk body a truly empty (or whitespace-only) line is an empty context line under the T212 lookahead (a consecutive blank run belongs to the hunk when the first non-blank after the run is a body line, otherwise the run is a separator); author a blank insertion or deletion with an explicit `+`/`-` prefix on an otherwise empty line, never a bare empty line. A blank-mismatch `GIT_HUNK_CONFLICT` names the prefix requirement in its suggestion. Removing a line whose text starts with `-` (e.g. a Markdown `- foo` bullet) needs a doubled `-- ` body-line prefix (diff `-` plus content `- foo`); a ` - foo` line is context and either fails the count gate or applies as a no-op.

A file section touched only by ordinary hunks is language-agnostic
(T137): the engine performs no Rust parse of the original file and no
Rust reparse of the rewritten file, so Markdown, Cargo.toml / .toml configs, and any other
plain text flow through the same tool. Any section whose file carries
>=1 RSmPL hunk requires the target to parse as Rust (Section 25.2).
Exception: when the patch carries `# follow=uses`, an ordinary-only
`.rs` target is parsed for follow search only — the rewritten file
still skips the reparse, so the ordinary bypass holds for it; an
ordinary-only target that does not parse stays unsearched and its
`old` mentions ride the unparseable tripwire (Section 14.8).

---

## 12. RSmPL Hunk Body Syntax

An RSmPL hunk body uses diff prefixes:

| Prefix | Meaning in RSmPL hunk |
|---|---|
| space | semantic context pattern |
| `-` | AST pattern to remove |
| `+` | AST pattern to insert |
| `\` | special diff marker, such as no newline marker |

Example:

```diff
@@ -10,7 @@ rule=rename_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }
```

The content after the diff prefix is parsed as Rust-pattern syntax.

### 12.1 Context lines

Context lines constrain the match but are not removed or added.

Example:

```diff
@@ -20,10 @@ rule=rewrite_inside_impl
# lang=rust
  impl Service {
      ...
-     fn old(...) {
+     fn new(...) {
          ...
      }
  }
```

### 12.2 Removed lines

Removed lines describe AST pattern fragments to delete or replace.

Example:

```diff
- old_api(E)
```

### 12.3 Added lines

Added lines describe AST fragments to insert.

Example:

```diff
+ new_api(E)
```

### 12.4 Pattern body is not a literal line diff

RSmPL hunk bodies are semantic patterns. The number of `-` and `+` pattern lines does not need to equal the old and new hunk line counts.

Example:

```diff
@@ -100,50 @@ rule=move_large_function
# item I
# bind=MovedFunction
- I
```

A hunk whose directive block is followed by zero body lines is rejected
with `RSMPL_PATTERN_CONFLICT` (rule-only envelope, verbatim reason +
envelope in Section 25.3) — an empty pattern is an authoring typo, never
a no-op.
This may remove a 50-line function using one pattern line.

---

## 13. Rust Pattern Language

RSmPL patterns are written in Rust-like syntax with metavariables and ellipses.

### 13.1 Concrete syntax

Concrete Rust syntax matches concrete Rust syntax.

Pattern:

```rust
old_api(x)
```

matches call expressions with the path `old_api` and argument `x`, subject to normal Rust parsing and optional semantic resolution.

### 13.2 Metavariables

Metavariables match AST fragments.

Example:

```diff
@@ -40,8 @@ rule=rename_calls
# expr E
- old_api(E)
+ new_api(E)
```

`E` matches any Rust expression in the argument position.

### 13.3 Ellipsis

`...` matches irrelevant syntax in specific syntactic positions.

Example:

```rust
fn foo(...) {
    ...
}
```

The parameter-list ellipsis matches any parameter list. The body ellipsis matches any function body.

### 13.4 Rust parse contexts

The engine must infer the parse context from the pattern.

Examples:

| Pattern | Parse context |
|---|---|
| `fn foo(...) { ... }` | item |
| `struct Old` | item rename |
| `old_api(E)` | expression |
| `let x = E;` | statement |
| `Some(P)` | pattern or expression depending on directive/context |
| `T` | type, path, or identifier depending on metavariable declaration |
| `type A = Vec<T>;` | type-alias RHS evolution (name preserved, Section 14.8) |
| `const MAX: i32 = E;` | const/static value evolution (name + ascribed type preserved, Section 14.8) |

> **v0.1 implementation note:** root inference is heuristic. A single `fn`-shaped minus/plus pair is a function signature rewrite (rename and/or slot evolution); a bare `<kind> <Name>` pair for `struct`, `enum`, `trait`, `union`, `type`, `const`, `static`, `mod`, `impl`, or `macro_rules!` is an item rename (Section 14.8); a `type NAME = RHS;` pair is a type-alias RHS evolution and a `const`/`static [mut] NAME: TY = EXPR;` pair is a const/static value evolution (Section 14.8); a bare `use <prefix>` pair is a use-path prefix rename (Section 14.9); a single bare declared metavariable (`- I`) is an item capture; an empty minus side is an insertion; `impl`-header context plus a function pair is an impl-method rename; `struct S {` context plus an `old: T` / `new: T` pair is a field rename; `enum E {` context plus a bare `Old` / `New` pair is a variant rename; `trait`-header context plus a function pair is a trait-method rename (`unsafe trait`/`unsafe impl` headers route identically — the fn-block scan skips block headers despite the `unsafe ` prefix) (same slot planner: `;` declarations and default bodies match, bodies/`;` preserved, explicit `pub` never matches — trait methods carry no visibility — and a `+` side writing it fails fast; the header names the trait ident, otherwise the block is accepted only on range overlap like `impl`); `NAME!(...)` shapes are macro patterns; anything else call-like is an expression rewrite. A bare `impl Name` minus whose plus side is not a bare `impl New` pair is not an item rename: when both sides parse as Rust types the hunk falls through to the general type matcher (so `- impl Display` / `+ Box<dyn Display>` rewrites `impl Trait` bounds structurally); anything else stays a pattern error. `# root=item` forces item capture, `# root=expr` forces expression rewrite, `# root=stmt` forces a `let`-statement pattern, and `# root=type` forces a type pattern for ambiguous shapes. Non-call expression shapes, `let` statements, and type shapes are matched by the generalized matcher (see the v0.1 note in Section 18).

The trait-method visibility gate above fails fast with `RSMPL_PATTERN_CONFLICT` (exit 2; `rule:` + `file:` lines; text omits the null `range:`/`suggestion:` lines, JSON carries explicit `range: null` / `suggestion: null`):

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: src/lib.rs
reason: trait methods cannot declare visibility: drop `pub` (or `pub(...)`) on the `+` side
```

If the parse context is ambiguous, the engine may require an explicit directive:

```text
# root=expr
```

or:

```text
# root=item
```

Recommended root directives:

```text
# root=item
# root=expr
# root=stmt
# root=type
```

Any other `# root=` value (`pat`, `impl_item`, `trait_item`, or a typo)
is rejected with `RSMPL_PATTERN_CONFLICT` — only the four force paths above
are implemented in v0.1, and silently falling through to heuristic routing
would violate Section 2.6. The remaining parse contexts are future work.
The `# root=item` path is strict the same way: a minus side that is not a
single declared metavariable fails fast instead of falling through to
heuristic routing, which would silently ignore the directive (the same
Section 2.6 violation — a call-shaped minus would otherwise apply as a
call rewrite with `# root=item` doing nothing).

Engine-verbatim (exit 2; `bogus` is the offending value, backticked):

```text
RSMPL_PATTERN_CONFLICT
rule: x
file: a.rs
reason: unknown `# root=` value `bogus` (only `item/expr/stmt/type` are supported in v0.1; remove it or use a supported value — see SPEC 13.4)
```

The failure carries `rule:`/`file:` but no `old_range:`/`suggestion:` lines
(null fields are omitted from text output; the JSON envelope carries
explicit `range: null` / `suggestion: null`).

The `# root=item` mismatch fails the same way (exit 2; `rule:` + `file:`;
text omits the null `range:`/`suggestion:` lines, JSON carries explicit
`range: null` / `suggestion: null`) — the sibling of the C3/C4/C5
mismatches for the fourth force path:

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: src/lib.rs
reason: `# root=item` requires a single-metavar item-capture pattern (write `- I` for a declared `# item I` metavariable)
```

Impl/trait-context header values are filters: the engine parses the
header once and compares it structurally against each candidate
block — `quote!`-normalized, so authored spacing never matters. An
`impl` header matches only the `impl` block with the equal self type
and equal trait path (inherent impls: no trait half on either side);
a trait-impl header (`impl Tr for S`) requires both halves equal; a
`trait` header matches only the trait block with the equal ident and
equal generics. Kind halves never mix. The comparison applies under
every scope, including `# scope=file` with `@@ -1,1 @@`: to rewrite
methods in K blocks, write K hunks (one header per block — the
`diffrs` slot/sig-evolution emitters do exactly this, sharing one
rule name). A header that parses as neither `impl` nor `trait`
matches nothing: the hunk fails loud with zero-match instead of
guessing a block. Copy the header verbatim from the file (modulo
whitespace); a mistyped header rewrites nothing, never the wrong
block.

---

## 14. Function Item Matching

Function matching is intentionally strict about modifiers.

### 14.1 Core rule

This pattern:

```rust
fn foo(...) {
    ...
}
```

matches only plain functions named `foo`.

It matches:

```rust
fn foo() {}
```

and:

```rust
fn foo(x: i32) -> i32 {
    x + 1
}
```

It does not match:

```rust
pub fn foo() {}
```

It does not match:

```rust
async fn foo() {}
```

It does not match:

```rust
unsafe fn foo() {}
```

It does not match:

```rust
fn foo<T>(x: T) {}
```

It does not match:

```rust
fn foo<T>(x: T)
where
    T: Clone,
{
    x
}
```

It does not match:

```rust
pub async unsafe fn foo<T>(x: T)
where
    T: Clone,
{
    x
}
```

#### 14.1.1 Terse rename

The bare pair:

```diff
@@ -10,1 @@ rule=rename_foo
# lang=rust
- fn foo
+ fn bar
```

is a rename-only shorthand for today's `fn foo(...) { ... }` form, usable
only when neither side writes slot or body text. It matches any function
named `foo` regardless of parameter list, return type, generic parameters
or where clause, and rewrites only the name ident — every other slot and
the body are preserved byte-for-byte, so `pub async fn foo<T>(x: T) -> T
where T: Clone { x }` becomes `pub async fn bar<T>(x: T) -> T where T:
Clone { x }`. Any visibility/qualifier tokens written before `fn` (e.g.
`- pub async fn foo`) still match literally, exactly as in the long form.
The terse pair takes no body lines: trailing `...`/`{`/`}` context lines
fail fast with a repair needle (write the bare pair, or spell both sides
long). A terse side paired with a long side fails fast the same way —
slot evolution needs the long form on both sides.

#### 14.1.2 Name holes

A bare `N` in the name position of a function pattern is a name hole: it
binds the site's ident text per site and replays the bound name on the
plus side while the written slots evolve around it — so one hunk evolves
headers across differently-named functions:

```diff
@@ -1,1 @@ rule=add_pub
# sweep
- V Q fn N(...) {
+ pub fn N(...) {
      ...
  }
```

`# ident X` declares any other name as a hole in this position
(`- V Q fn Foo(...)` / `+ pub fn Foo(...)` with `# ident Foo`); diet `N`
needs no declaration. Slot strictness is unchanged: the minus side's
written slots must still match (concrete `fn N<T>(...)` requires exactly
`<T>`), and the same no-op discipline applies — a site already in the
plus state is reported with a byte-identical `rename`-labeled edit that
contributes no diff lines, and a sweep where every site already conforms
fails loud through the sweep change gate (Section 24.6). A plus-side `N`
with no minus-side `N` is unbound (`fix_pattern`, code `unbound`).

Method positions share the spelling: an `impl`/`trait`-context hunk
(Section 13.4) may write `fn N(...)` for the method name with the same
bind-and-replay semantics. `N` never binds qualified paths and never
appears in call/method callee positions (Section 18.6 C20); to match a
literal ident spelled `N`, write the concrete name (call shapes match
literally there).

Bare-pair item positions share the spelling for attr-only evolution
(WI-10b): `- struct N` / `+ struct N` with an attribute delta binds
every site's name and replays it byte-identical (the attr edits carry
the change; vis/generics/bodies are preserved like renames). Holes
must appear on both sides — a hole on one side only fails fast (a
minus-hole with a concrete plus would rename every item to one name),
as does a hole pair with no attr delta (the rename-usage error) or
with `# follow=uses` (holes preserve every name; there is nothing to
follow). `$`-prefixed spellings keep the existing rejection (write
bare `N`, not `$N`).

### 14.2 Function slots

A Rust function item is modeled as these slots:

```text
attributes
visibility
qualifiers
fn keyword
name
generic parameters
parameter list
return type
where clause
body
```

For the pattern:

```rust
fn foo(...) {
    ...
}
```

the required slot interpretation is:

| Slot | Required interpretation |
|---|---|
| attributes | preserved/unconstrained by default |
| visibility | absent |
| qualifiers | absent |
| `fn` keyword | present |
| name | exactly `foo` |
| generic parameters | absent |
| parameter list | any |
| return type | any |
| where clause | absent |
| body | any |

The return type is intentionally unconstrained by this shorthand, because both of these should match:

```rust
fn foo() {}
```

```rust
fn foo() -> i32 {
    1
}
```

To constrain the return type, write it explicitly.

Example:

```rust
fn foo(...) -> i32 {
    ...
}
```

To require no explicit return type, use:

```text
# returns=none
```

Example:

```diff
@@ -10,7 @@ rule=rename_unit_foo
# returns=none
- fn foo(...) {
+ fn bar(...) {
      ...
  }
```

The directive works for free functions and for methods: an
impl/trait-context hunk carrying `# returns=none` requires the target
method to carry no explicit return type (Section 13.4 shares the slot
planner).

 On a non-function hunk the directive fails fast with
 `RSMPL_PATTERN_CONFLICT` (rule + file lines; text omits the null
 range/suggestion lines, JSON carries explicit nulls):

 ```text
 RSMPL_PATTERN_CONFLICT
 rule: r
 file: src/lib.rs
 reason: `# returns=none` requires a function-item pattern
 ```

### 14.3 Explicitly matching visibility

To match a public function, write `pub`:

```diff
@@ -10,7 @@ rule=rename_pub_foo
# lang=rust
- pub fn foo(...) {
+ pub fn bar(...) {
      ...
  }
```

To match `pub(crate)`:

```diff
@@ -10,7 @@ rule=rename_pub_crate_foo
# lang=rust
- pub(crate) fn foo(...) {
+ pub(crate) fn bar(...) {
      ...
  }
```

To abstract over visibility, declare a visibility metavariable:

```diff
@@ -10,7 @@ rule=rename_any_visible_foo
# visibility V
- V fn foo(...) {
+ V fn bar(...) {
      ...
  }
```

### 14.4 Explicitly matching qualifiers

Qualifiers include:

```text
const
async
unsafe
extern
extern "ABI"
```

To match an async function:

```diff
@@ -10,7 @@ rule=rename_async_foo
# lang=rust
- async fn foo(...) {
+ async fn bar(...) {
      ...
  }
```

To match an unsafe function:

```diff
@@ -10,7 @@ rule=rename_unsafe_foo
# lang=rust
- unsafe fn foo(...) {
+ unsafe fn bar(...) {
      ...
  }
```

To match an async unsafe function:

```diff
@@ -10,7 @@ rule=rename_async_unsafe_foo
# lang=rust
- async unsafe fn foo(...) {
+ async unsafe fn bar(...) {
      ...
  }
```

To abstract over qualifiers:

```diff
@@ -10,7 @@ rule=rename_qualified_foo
# qualifier_list Q
- Q fn foo(...) {
+ Q fn bar(...) {
      ...
  }
```

### 14.5 Explicitly matching generics and where clauses

A plain function pattern does not match generic functions.

To match a specific generic shape:

```diff
@@ -20,10 @@ rule=rename_generic_foo
# lang=rust
- fn foo<T>(...)
+ fn bar<T>(...)
  {
      ...
  }
```

To match a generic function with a where clause:

```diff
@@ -20,12 @@ rule=rename_where_foo
# lang=rust
- fn foo<T>(...) where T: Clone {
+ fn bar<T>(...) where T: Clone {
      ...
  }
```

The `where` clause lives on the `fn` pattern line: `- fn foo<T>(...) where T: Clone {`. A pattern line without `where` requires its absence (like every other slot, Section 14.1) — the multi-line `where`-as-context spelling never constrains the slot. The signature must be terminated by `{` (which may sit on the `fn` line or on a trailing `where` line) or `;`: an unterminated tail swallows the body `...` into the slot text and fails fast with a missing-brace needle.

Engine-verbatim missing-brace rejections (all three exit 2; all carry `rule:`/`file:`; text omits the null `range:`/`suggestion:` lines, JSON carries explicit `range: null` / `suggestion: null`):

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: src/lib.rs
reason: unexpected trailing tokens in fn pattern: `...`
```

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: src/lib.rs
reason: fn pattern `where` clause is missing its opening `{` (or `;` for declarations): the signature swallowed the body lines — put the brace on the signature line (SPEC 14.5)
```

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: src/lib.rs
reason: fn pattern `->` return type is missing its opening `{` (or `;` for declarations): the signature swallowed the body lines — put the brace on the signature line (SPEC 14.5)
```

The first reason echoes the swallowed tail text (here the body `...` line); the second fires when the `fn` line carries a `where` clause but no opening brace or `;`, the third when it carries a `->` return type but no opening brace or `;`.

To abstract over generic parameters and where clauses:

```diff
@@ -20,12 @@ rule=rename_any_generic_foo
# generic_params G
# where_clause W
- fn foo G (...) W {
+ fn bar G (...) W {
      ...
  }
```

The syntax `G` and `W` are RSmPL metavariables, not literal Rust tokens.

### 14.6 Attributes, doc comments, and attached metadata

Attributes are handled differently from visibility, qualifiers, generic parameters, and `where` clauses.

A plain pattern does **not** require attached attributes to be absent. Instead, attributes are treated as an **attached metadata envelope** around the matched AST node when that Rust AST node kind supports outer attributes. The default policy is:

```text
# attrs=preserve
```

This policy applies to any Rust AST node category that can carry attributes, not only items. In addition to functions, structs, traits, modules, and impl blocks, it applies to supported non-item nodes such as statements, expression statements, block expressions, match arms, struct fields, enum variants, and other Rust syntax nodes whose grammar permits attributes.

If the matched AST node kind cannot carry attributes, `# attrs=preserve` has no effect. If a pattern explicitly writes attributes in a syntactic position where Rust does not allow them, the pattern must fail to parse.

Under `# attrs=preserve`, this pattern:

```rust
fn foo(...) {
    ...
}
```

matches all of the following, assuming the other explicitly written function slots match:

```rust
fn foo() {}
```

```rust
#[inline]
fn foo() {}
```

```rust
/// Adds one to the input.
fn foo(x: i32) -> i32 {
    x + 1
}
```

Rust doc comments such as `/// ...` and `//! ...` are represented in Rust's AST as `#[doc = "..."]` attributes. Therefore, for matching and movement, doc comments are part of the same attached metadata envelope as ordinary attributes.

This rule exists for LLM usability. LLMs frequently omit doc comments when describing a refactor. A pattern should not fail merely because the target function has documentation.

However, this relaxed default applies only to attributes and doc comments. It does **not** relax visibility, qualifiers, generic parameters, or `where` clauses. For example, `fn foo(...) { ... }` still does not match `pub async fn foo<T>(...) where T: Clone { ... }`.

#### Attribute preservation on non-item nodes

When a `stmt`, `expr`, `field`, or other non-item metavariable captures a Rust AST node that supports attributes, the capture includes the attached metadata envelope under `# attrs=preserve`.

Example source:

```rust
#[allow(clippy::let_unit_value)]
let x = compute();
```

Patch:

```diff
@@ -10,1 @@ rule=rename_stmt_initializer
# stmt S
# expr E
- let x = E;
+ let y = E;
```

Required result:

```rust
#[allow(clippy::let_unit_value)]
let y = compute();
```

If the replacement pattern explicitly supplies its own attributes, the implementation must follow the active `# attrs=...` policy and reject ambiguous merges rather than silently duplicating or dropping attributes.

Engine-verbatim ambiguous-merge rejections (both exit 2; both carry `rule:`/`file:` plus a set `suggestion:` line — text carries it, JSON explicit; `range:` is null in both):

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: src/lib.rs
reason: replacement writes attributes the pattern did not match; write the same attributes in the `-` pattern or drop them
suggestion: mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`
```

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: src/lib.rs
reason: replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges
suggestion: mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`
```

The first fires when the `+` side writes attributes the `-` side did not match; the second when both sides write attributes that differ. These two are the only 14.x compile-time fences with a set suggestion (like the Section 9.6 `repeat`-value entry); the match-time fence below is the only other 14.x suggestion carrier.

Match-time attribute failures differ from the compile-time merge rejections above: a candidate that carries attributes the active policy forbids fails with `RSMPL_MATCH_CONFLICT` (exit 2; all carry `rule:` + `file:`; text omits the null `old_range:` line and, except where noted, the null `suggestion:` line; JSON carries explicit `range: null` / `suggestion: null`-or-string):

```text
RSMPL_MATCH_CONFLICT
rule: r
file: src/lib.rs
reason: function has attributes but `# attrs=absent` requires none
```

```text
RSMPL_MATCH_CONFLICT
rule: r
file: src/lib.rs
reason: item has attributes but `# attrs=absent` requires none
```

```text
RSMPL_MATCH_CONFLICT
rule: r
file: src/lib.rs
reason: statement has attributes but `# attrs=absent` requires none
```

The `statement` reason echoes the active policy (`absent` or `explicit`); `use` items report with the `item` wording (they route through the item matcher). Contrast with the silent skip: under default `preserve`, a function/item pattern whose written `#[...]` matches no site's envelope skips the candidate (`no match found`), while the general matcher hard-fails instead:

```text
RSMPL_MATCH_CONFLICT
rule: r
file: src/lib.rs
reason: attached attributes do not match explicitly written pattern attributes
suggestion: use `# attrs=preserve` (default) to ignore attributes
```

The suggestion is set on the function/item-`explicit` and general-`preserve` paths; a general-`explicit` (or `absent`) mismatch carries the same reason with a null suggestion.

#### Attribute preservation on rewrite

When an attributed AST node is rewritten and the replacement pattern does not explicitly mention attributes, the engine must preserve the original attached attributes and doc comments.

Example source:

```rust
/// Adds one.
#[inline]
fn foo(x: i32) -> i32 {
    x + 1
}
```

Patch:

```diff
@@ -10,8 @@ rule=rename_documented_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }
```

Required result:

```rust
/// Adds one.
#[inline]
fn bar(x: i32) -> i32 {
    x + 1
}
```

#### Attribute preservation on move

When an `item` metavariable captures an item, the capture includes the attached metadata envelope by default. This allows functions, structs, traits, modules, impl blocks, and other items to be moved together with their doc comments and attributes.

Example source:

```rust
/// Shared helper.
#[inline]
fn helper(x: i32) -> i32 {
    x + 1
}
```

Patch:

```diff
@@ -20,6 @@ rule=cut_helper
# item I
# bind=Helper
- I
```

The binding `Helper` contains both the function item and its attached doc comments and attributes.

#### Explicitly constraining attributes

To require a specific attribute, write it explicitly:

```diff
@@ -10,8 @@ rule=rename_inline_foo
# lang=rust
- #[inline]
- fn foo(...) {
+ #[inline]
+ fn bar(...) {
      ...
  }
```

To bind and preserve arbitrary attributes explicitly, write nothing:
under the default `# attrs=preserve` an unwritten pattern already keeps
each site's own attributes. v0.1 has no attribute-list abstraction —
`# attributes A` is rejected with `RSMPL_PATTERN_CONFLICT` (write the
`#[...]` lines literally on both sides to constrain them, or omit
them).

Written attributes constrain the match even under the default preserve
policy: a minus side naming `#[inline]` matches only sites carrying
exactly those attributes (other sites are skipped, like any slot
mismatch). The plus side must mirror the minus side exactly, except for
the WI-10a Preserve delta: written lines are the delta — a plus-only
line adds it, a minus-only line removes it, and same-length differing
lines replace 1-to-1 in written order above the item line (unwritten
attached attrs are preserved; already-plus sites skip an add; remove/
replace require the minus lines present). The delta covers `let`
statements too (WI-10b): attr edits land above the `let` line while the
body edit shrinks to the `let` keyword, so the ranges stay disjoint.
Differing non-empty lengths stay a pattern error, as does any delta
under `# attrs=explicit` / `# attrs=strip` (mirror the `#[...]` lines,
or use `# attrs=strip`).

To require that no attributes or doc comments are attached, use:

```diff
@@ -10,7 @@ rule=rename_unattributed_foo
# attrs=absent
- fn foo(...) {
+ fn bar(...) {
      ...
  }
```

Supported attribute policies are:

| Directive | Meaning |
|---|---|
| `# attrs=preserve` | Default. Ignore unwritten attached attributes for match eligibility and preserve them across rewrite, deletion capture, and movement for any AST node kind that supports attributes. Explicitly written attributes constrain the match (exact match, other sites skipped) and the plus side must mirror the minus side. |
| `# attrs=absent` | Require no attached attributes or doc comments. |
| `# attrs=explicit` | Only attributes written in the pattern participate. Unwritten attached attributes cause the match to fail. |
| `# attrs=strip` | Match like `preserve`, but do not copy attributes to the replacement. This must be rejected unless the replacement target can legally omit the attributes. |

`# attrs=strip` is intentionally explicit because accidentally deleting documentation or attributes is high-risk.
Non-default `# attrs=` on call/method-call/macro shapes is rejected outright (the matchers never read the policy; envelopes live on the enclosing statement) — remove the line.

Doc-comment mirrors (`///`, `//!`, and single-line `/** ... */`) normalize to `#[doc = "..."]` on any pattern path that accepts mirrored attributes; multi-line block docs cannot be mirrored — use an ordinary hunk.

### 14.7 Signature evolution

A function hunk whose `-` and `+` sides differ in more than the name evolves each explicitly written slot independently. Slots not written differently are preserved byte-for-byte (including the body, which is never rewritten). The reference implementation compares slots whitespace-insensitively against target text sliced from the original source:

| Slot | `-` matches target when | `+` effect when different from `-` |
|---|---|---|
| name | equal, or `$Meta` binds any, or the `N` / `# ident` name hole (Section 15.8) binds any | replaced with the `+` name (`$Meta` and name holes preserve) |
| visibility | absent requires inherited; concrete requires normalized equality; `V` matches any | insert/replace/delete `pub…` (`V` on `+` preserves) |
| qualifiers | same set (with `extern "ABI"`); `Q` matches any | insert/replace/delete qualifier run |
| generics | absent requires none; concrete requires normalized equality; `G` matches any; angle text containing `...` is a pattern error | insert after name / replace `<…>` / delete |
| parameters | `...` matches any; concrete (incl. `()`) requires normalized equality; mixing `...` with other tokens on either side is a pattern error | `...` preserves; concrete replaces the parenthesized list |
| return type | `...`-shorthand (no `->`) matches any; absent (via `# returns=none`) requires none; concrete requires normalized equality; `-> ...` is a pattern error (omit `->` for any) | `+` with no `->` preserves, except minus-concrete/`+`-omitted deletes `-> T`; concrete inserts after `)` or replaces `-> T` |
| where clause | absent requires none; concrete requires normalized equality of the predicates; `W` matches any; predicates containing `...` are a pattern error | insert before `{` / replace / delete (minus-concrete/`+`-omitted) |
| body | must be exactly `...` (or `;`) on both sides; mixing `...` with other tokens is a pattern error | always preserved, except one to three joint body pairs (see below); any other concrete body is a pattern error |

Trailing commas are ignored when comparing parameter lists and `where` predicates: a rustfmt-canonical multi-line target (`x: i32,`, `T: Clone,`) matches the single-line pattern spelling (`x: i32`, `T: Clone`), and a comma-only slot difference never emits an edit — the target text is preserved.

A rename+body joint (WI-20, `let`-single widened WI-13 slice 1, two-pair widened WI-13 slice 2, three-pair widened WI-13 slice 3, slot-evolution triple widened WI-13 slice 4) is the one body exception: a hunk carrying the header pair in the single-line `{ ... }` form plus one to three extra `-`/`+` body line pairs — each pair both sides one `syn::Expr` (e.g. `- x + 1` / `+ x + 2`), or both sides one plain same-binder `let` statement with an init (`syn::Stmt::Local`, binder `pat` token-equal, no attributes, e.g. `- let y = 1;` / `+ let y = 2;`) — rewrites the header plus those whole lines inside the renamed definition's body only (indent preserved, one `body`-labelled edit per pair). The header is rename-only (slices 1–3) or carries a Joint-A-shaped slot evolution (slice 4 triple: evolving ⊆ {vis, qual, params, ret} with params or ret evolving, at most two evolve, generics/where preserved; the engine applies it through the shared header splice beside the `body` edits). Each body minus must occur exactly once in the file as a whole line inside the renamed body; zero, multiple, or out-of-body hits fail loud per pair (`RSMPL_MATCH_CONFLICT`, never a sweep). Non-Joint-A-shaped slot-evolving headers (generics evolution, three slot evolutions, vis-only evolution, B1-shaped arg evolutions), attribute deltas, binder-changing `let` pairs, `let` pairs without an init, four-or-more body pairs, multi-line bodies, and twin in-body hits stay on the existing error paths.

```diff
@@ -1,2 @@ rule=r1
# sweep
# follow=uses
- fn foo(x: i32) -> i32 { ... }
+ fn bar(x: i32) -> i32 { ... }
- x + 1
+ x + 2
```

The five slot/body/param misuses above fail fast with `RSMPL_PATTERN_CONFLICT`, engine-verbatim (all exit 2; all carry `rule:`/`file:`; text omits the null `range:`/`suggestion:` lines, JSON carries explicit `range: null` / `suggestion: null`; suggestion is null throughout):
 ```text
 `...` in a function parameter list must stand alone: write `(...)` to match any parameters or fully-concrete parameters (SPEC 14.7)
 `...` is not supported as a return type in v0.1: omit `->` to match any return type or write a concrete type (SPEC 14.7)
 `...` is not supported in generic parameter lists in v0.1: write concrete `<T>` or abstract with `# generic_params G` + `fn foo G (...)` (SPEC 14.7)
 `...` is not supported in where clauses in v0.1: write concrete predicates or abstract with `# where_clause W` + `... W ...` (SPEC 14.7)
 v1 function hunks require `...` bodies on both sides (body evolution is not supported; the original body is always preserved)
 ```

 Structural fn-pattern rejections beyond the slot/`...` five fail fast the same way (`RSMPL_PATTERN_CONFLICT`, exit 2, `rule:`/`file:` envelope; text omits the null `range:`/`suggestion:` lines, JSON carries explicit `range: null` / `suggestion: null`; suggestion is null throughout), engine-verbatim:
 ```text
 fn pattern must contain `fn` keyword, got `{text}`
 fn pattern missing function name
 fn pattern missing parameter list
 unbalanced generics in fn pattern
 unbalanced parameter list in fn pattern
 unbalanced body block in fn pattern
 unbalanced attribute in fn pattern
 unbalanced pub(...) in fn pattern
 `->` without return type in fn pattern
 function pattern bodies must be exactly `...` in v0.1 (body evolution is not supported; the original body is always preserved)
 block comments (`/* ... */`) are not supported in patterns in v1 (mirror doc comments with `#[doc = "..."]` or use an ordinary hunk)
 ```

 The `got` backtick echoes the full minus pattern text with newlines preserved. `missing function name` needs the degenerate single-line spelling (any body lines supply a name token, and a bare `fn` with body lines routes to the general-`...` C1 fence instead); `missing parameter list` fires when a name or generics are present but no `(` follows. `exactly ...` fires on mixed bodies (`{ ..., x; }`); a wholly missing body fires the `v1 function hunks require ...` fence above instead. Single-line `/** ... */` doc lines normalize to `#[doc = "..."]` (T156); only multi-line or ordinary block comments hit the needle. The unterminated `` `->` return type `` twin lives in Section 14.5.

Example (add visibility, generics, and a return type in one hunk, params and body preserved):

```diff
@@ -1,2 @@ rule=evolve
# lang=rust
- fn compute(...) {
+ pub async fn compute<T>(...) -> Vec<T> {
      ...
  }
```

#### Raised joints (`fn_joint`)

`diffrs` raises a rename+evolution joint as one intent (SPEC D.4):
exactly one column-0 `fn` definition renamed while its header evolves,
joined with its call-callee uses in the same file. Three shapes fire.
Joint-A (rename+vis/qual/params/return with at most two slots evolving
and callee-only calls) emits the single header-pair hunk with `# sweep`
+ `# follow=uses` carrying both deltas. Joint-B1
(rename+params-evolve with uniform arg-adding, uniform
trailing-drop, or uniform trailing-retype calls) and
Joint-B2 (rename+attr add/remove with uniform callee-only calls) emit
two hunks under one shared rule — the def hunk (header pair rebuilt
from native lines, attr line on its own `+`/`-` line for B2) plus the
call-template hunk (`- foo(E0)` / `+ bar(E0, 0)`) — with `# sweep` and
no follow (a follow would claim the call hunk's sites and the lowering
would refuse the overlap). The authored spelling is the same pair an
agent would write by hand (explicit slots, `{ ... }` body). Anything
else in the joint shape (body delta, hetero call shapes,
generics/where delta, vis-only/qual-only or triple-slot
delta, twin defs, N=1 B2/Body joints) suppresses the old callee
name for the file, so the section stays wholly ordinary — the
fragmented fallback (ordinary definition plus a rename-claiming call
sweep) is unrepresentable by construction. Multi-file joints stay
ordinary (future work).
Joint-B1 fires at N>=1: the single-call two-hunk emission prices the
WI-7 precision allowance over the proved site set.

#### Slot removal

Omitting a slot on the `+` side deletes it **only when the `-` side
names the slot concretely**. A wildcard minus (`...` parameters, no
`->` written, no `where` written) plus omission always preserves — this
is the rename fast-path safety rule: a bare `fn foo(...)` → `fn
bar(...)` must never silently drop return types or where clauses.

```diff
@@ -1,1 @@ rule=strip_result
# lang=rust
- fn compute(...) -> Result<Vec<T>, E> {
+ fn compute(...) {
      ...
  }
```

The agent names the concrete `-> Result<Vec<T>, E>` on the minus side
(it has read the file to cite the range) and omits it on the plus
side. There is deliberately no wildcard-minus removal spelling: match
any return type *and* delete it would require a second removal syntax
for a corner case (Rust has no overloading, so multi-target stripping
is rare), and `# returns=none` stays a minus-side constraint (requires
absence; pairs with a concrete plus side for insertion).

> **v0.1 implementation note:** evolved slots are emitted with canonical spacing (`<T>`, `x: i32`, `where T: Clone`); only `# format=preserve` is implemented, so no `rustfmt` normalization pass runs — run `cargo fmt` after apply (Section 22.2). Deleting a slot absorbs one following space to avoid doubled spacing.

### 14.8 Other item renames

Function renames (14.1–14.7) are strict about explicitly written slots because signatures evolve. Renaming any other item kind is rename-only with one exception: a rename may evolve exactly the visibility slot by one closed-set step (`- pub struct Old` / `+ pub(crate) struct New`, WI-5-R2); otherwise the author writes a bare `<kind> <Name>` pair and the engine replaces exactly one name-ident span per match, preserving visibility, generics, bodies, attributes, and formatting byte-for-byte (the same fast-path as a name-only function rename).

Example (rename a struct without restating its fields):

```diff
@@ -1,3 @@ rule=rename_struct
# lang=rust
- struct Old
+ struct New
```

Supported kinds:

| Kind | Bare pattern | Also accepted (ignored, must stay `...`/empty) |
|---|---|---|
| `struct` | `struct Old` | `struct Old;`, `struct Old(...)`, `struct Old { ... }` |
| `enum` | `enum Old` | `enum Old { ... }` |
| `trait` | `trait Old` | `trait Old { ... }` |
| `union` | `union Old` | `union Old { ... }` |
| `type` | `type Old` | `type Old = ...;` |
| `const` | `const Old` | `const Old: ... = ...;` |
| `static` | `static Old` | `static mut Old`, `static Old: ... = ...;` |
| `mod` | `mod old` | `mod old;`, `mod old { ... }` |
| `impl` | `impl Old` | `impl Old { ... }` (self-ty and trait-side, see below) |
| `macro_rules!` | `macro_rules! Old` | `macro_rules! Old { ... }` (definition ident only, body preserved verbatim) |

Rules:

- Both sides must name the same kind; a kind mismatch (`struct` → `enum`) is a pattern error. Identical minus/plus names are a pattern error, except for the WI-10a Preserve attr-evolution pair (same name with differing written attr sets classifies as an attr delta, not a rename).
- An optional leading `pub…` / `unsafe` / `mut` prefix is accepted for matching convenience, but the minus and plus spellings must agree (whitespace-insensitive): `struct Old` matches `pub struct Old<T> { ... }` and preserves everything except the ident, and `pub struct Old` / `pub struct New` renames likewise. One closed-set vis step may ride a rename (`pub struct Old` / `pub(crate) struct New` — names differ, kinds equal, attrs mirror-equal, no qualifier delta; the engine rewrites the vis slot alongside the name, WI-5-R2). A same-name closed-set vis step is a pure vis evolution (`struct S` / `pub struct S` — same kind/name/suffix, vis differs within the closed set; the rename edit replays the ident while the vis edit carries the change). Any other prefix difference — a qualifier delta (`static Old` / `static mut New`), a vis step combined with an attr delta, or a vis outside the closed set (`""`, `pub`, `pub(crate)`, `pub(super)`, `pub(in <path>)`) — is a pattern error instead of a silent change. Prefix–kind legality is enforced: `unsafe` only on `trait`/`impl` headers, no visibility on `impl` (`unsafe struct`, `pub impl` are pattern errors), and `macro_rules!` definitions take neither visibility nor `unsafe` (`pub macro_rules! Old`, `unsafe macro_rules! Old` are pattern errors). The `!` between `macro_rules` and the name is required (`macro_rules Old` without it is not an item shape). Any other concrete text (generics, fields, types, values, `where`) is a pattern error directing the author to the bare form.
- `$Meta` abstracted names are not supported for item renames in v0.1 (write concrete names). Backtick literal quoting follows Section 15.4.
- `auto trait` headers are not supported in v0.1: the author gets a fail-fast pattern error directing to an ordinary hunk (auto traits are never renamed in practice; silently falling through to a no-match would violate Section 2.6).
  The engine reports these rejections verbatim (rule + file lines in text mode; text omits the null range/suggestion lines, JSON carries explicit nulls; suggestion is null throughout):
  ```text
  `auto trait` headers are not supported in v1 (use an ordinary hunk)
   item qualifier differs or is not preserved (`mut` vs `mut`); rename+visibility joints preserve `unsafe`/`mut` — write the same qualifier on both sides or neither
   rename+visibility+attribute is two evolving slots — split the attribute delta into its own hunk or use an ordinary hunk
  item rename kind mismatch: minus is `struct` but plus is `enum` (kinds must match)
  item rename minus and plus names are identical (`Old`); write the new name on the `+` side
  `unsafe` is only legal on `trait`/`impl` headers, not `struct` (write the bare `<struct> <Name>` form)
  `impl` blocks take no visibility (write the bare `impl <Name>` form)
  concrete item shape after `<struct> <name>` is not supported in v1 (write the bare `<struct> <Name>` form; visibility, generics and bodies are preserved)
  missing item name after `struct` (write a bare `<kind> <Name>` pattern)
  item rename plus side must be a bare `<kind> <Name>` pattern matching the minus kind
  item rename minus side must be a bare `<kind> <Name>` pattern matching the plus kind
  `$Meta` abstracted names are not supported for item renames in v1 (write concrete names)
  invalid item name `r#123` (expected an identifier)
  ```
  (`<kind>` in the kind-mismatch reason echoes the lowercased kind — probed `struct` + `enum`; the kind in the missing-name reason echoes the written kind. The kind in the `unsafe`-prefix and concrete-shape reasons echoes the written kind the same way — probed `struct` + `enum` for both (the fences above show the `<struct>` spelling as canonical; all nine kinds share the template). The two bare-side fences fire when one side is a recognized bare item but the other side is a different classifier's shape — probed minus `struct Old` / plus `foo(E)` and vice versa with a call-shaped other side, which also steers clear of the `impl`-type fallthrough that only fires when both sides parse as Rust types, Section 13.4. A concrete-shape or identical-name pair fires its own fence first, never reaching the bare-side gate; the `use` pair lives in Section 14.9.)

  The two lexical-name fences fire on either side (`- struct $Meta` or `- struct r#123`, same for `+`); `r#123` is the reachable invalid-name trigger (a bare `r#name` is the only legal raw-identifier spelling, Section 15.4). The item-local `unbalanced backtick quoting in item name` / `invalid backticked item name` arms are defensive-only: `validate_quoting` runs before classification, so the CLI emits the general Section 15.4 fences first (T287/T290/T291 precedent, documented not pinned).

  Item-rename attribute merges fail fast with item-specific wording (rule + file lines in text mode; text omits the null range/suggestion lines, JSON carries explicit nulls; suggestion is null — unlike the 14.6 pair, which carries a set suggestion):
  ```text
  plus side writes attributes the minus side omitted (ambiguous merge); write the attributes on both sides or neither
  ```
  The differ case needs no new fence: a minus/plus pair that both write attributes but differ emits the byte-identical Section 14.6 second fence (`replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges`, suggestion set). Both gates need own-line `#[...]` spellings — a single-line `#[derive(Clone)] struct New` plus side fails earlier at the classifier with the bare-pair fence above (`item rename plus side must be a bare ...`), never reaching the merge gate.
- `impl Old` renames the self-type ident and the trait-side path's last segment. It matches `impl Old`, `impl<T> Old<T>`, `impl Trait for Old`, and `impl Old for Foo`, rewriting only the `Old` spans while preserving generics, qualifiers, and the other side. Rationale: in valid Rust one name cannot be both a type and a trait in the same scope (type-namespace collision), so the bare pair is unambiguous; matching stays syntactic (Section 19). Generic (`impl Old<T> for Foo`), qualified (`impl crate::Old for Foo`), and negative (`impl !Old for Foo`) trait paths match on the last segment only.
- Matching reuses the function scope modes (`scope=contained/anchor/overlap/exact`, anchor on the ident line), attribute policies (`# attrs=...`), and `allow`/`optional` count validation. File-level, inline-`mod`-level, and block-level items are collected: the syntactic walk reaches items nested in fn bodies, closures, and method bodies, so a local `struct` renames like a top-level one (a top-level and a block-level same-named item need `# allow=many`, like any multi-match). Items inside macro bodies are not matched (Section 21). Each match emits one ident-span edit, so overlapping-match rules are trivially satisfied.

#### Use-site follow-ups

An item rename rewrites the definition header only — one ident-span edit
per match. Type annotations, value-position paths, struct literals, enum
variants, and `use` imports keep the old name while the hunk applies
cleanly, so a full rename composes follow-up hunks (all probe-verified):

| Site | Follow-up hunk |
|---|---|
| `x: Old`, `-> Old` | `- Old` / `+ New` with `# root=type` + `# allow=many` |
| unit value `Old` | `- Old` / `+ New` with `# root=expr` |
| `Old { a: 1 }` (concrete `m::Old { a: 1 }` matches the same spelling with the qualifier written out) | `- Old { a: E }` / `+ New { a: E }` (`# expr E`; write the qualifier concretely — a placeholder qualifier such as `E::Old` never matches, use one concrete hunk per qualifier) |
| `E::V(..)` | call-shape `- E::V(A)` / `+ E::W(A)` (`# expr A`) |
| `use …::Old` | use-path prefix `- use crate::Old` / `+ use crate::New` (`# allow=many`, Section 14.9) |

A bare `- Old` / `+ New` pair with no `# root=` never routes (ambiguous
shape, Section 13.4) — always force the context explicitly.

#### Use-site following (`# follow=uses`, WI-6a)

A fn or item rename hunk carrying `# follow=uses` rewrites the definition
ident **and** its same-file use sites in one hunk:

```diff
diff --git src/lib.rs
@@ -2,3 @@ rule=rename
# follow=uses
- struct Old
+ struct New
```

This rewrites the `struct Old` header plus every same-file use the
syntactic walk can prove: type paths (`x: Old`, `-> Old`, `Vec<Old>`),
value and call paths (every segment spelling `Old` — bare `Old`,
trailing `m::Old`, leading `Old::new`, middle `a::Old::assoc`;
turbofish lives outside the ident span and is preserved), struct-literal
and pattern paths (`Old { a }`, `Some(Old)`), `impl` self-ty and
trait-side segments (`impl Old`, which name the item itself), and
`use`-tree segments (`use m::Old`, group members, `as` tails preserved).
The hunk range is ignored for uses: a follow rewrites the whole file,
which is the point (no range discovery). Spans cover definition + use
edits (`spans.len() >= matches`; `matches` counts definitions); the
lowered diff is the observable truth as always (Section D.2).

  Composed definition pairs (WI-5 slice 1): a definition rename pair
  may carry exactly one attached attr delta written above it on its own
  diff lines (add/remove/replace under the default Preserve law); the
  pair still applies and verifies as one follow hunk, and raising emits
  that spelling (SPEC D.4). Two attr deltas, or an attr delta detached
  from its definition, stay ordinary.
  Evolution co-residents (WI-5 slice 2a): no composed spelling exists
  yet for a rename plus a member/type evolution, so a section where an
  evolution delta shares its native hunk with the rename's use raises
  nothing — the bare rename single is suppressed and the section stays
  wholly ordinary (the item analogue of the WI-4 claim-then-veto law).
   Composed rename+evolution joints (WI-5 slice 2b, first + 2b-next,
   first): a `struct` rename pair may carry exactly one attached
   field-type evolution pair, and an `enum` rename pair may carry
   exactly one attached variant-evolution pair (tuple payload with equal
   arity and ≥1 differing type, struct payload with equal field names
   and ≥1 differing type, or discriminant with differing exprs), each
   written below the def pair on its own diff lines (same member, no
   container header — the def pair names the container, the member pair
   rides it; `# follow=uses` allowed, the rename half owns uses).    Each
   hunk applies and verifies atomically (rename ident + member span(s)
   + follow in one matcher pass; twin containers veto at the raiser,
   never half-applied), and raising emits that spelling for same-file
   same-section same-container groups (at most one joint per def section;
   2b-next2a: the def pair and the member pair may ride distinct balanced
   hunks of one section, claimed together with the uses). Struct rename
   + field-addition joint (WI-5 slice 2b-next2b-first): a `struct` rename
   plus exactly one field-addition line in the same file and section —
   either a distant pure-insertion hunk (`@@ -N,0 @@`) or a hunk adjacent
   to the def change (one minus, two plus) — plus claimable uses raises
    to a shared-rule two-hunk emission (the WI-12b addition hunk naming
    the old container, then the terse rename follow hunk), proved by the
    one multi-file full-post probe (D13) under strict T375. Variant-add
    joint (WI-5 slice 2b-next2b-next): the enum half of the same ladder
    — an `enum` rename plus exactly one variant-addition line (unit,
    tuple-payload, struct-payload, or discriminant) in the same file and
    section, either a distant pure-insertion hunk or an adjacent
    (1-minus, 2-plus) hunk, plus claimable uses raises to the same
    shared-rule shape with the `enum Old {` context. Struct rename +
    field-deletion joint (WI-21 slice 1): a `struct` rename plus exactly
    one field-deletion line in the same file and section — either a
    distant pure-deletion hunk (`@@ -N +M,0 @@`) or a hunk adjacent to
    the def change (two minus, one plus) — plus claimable uses raises to
    a shared-rule two-hunk emission (the field-deletion hunk naming the
    old container, then the terse rename follow hunk), proved by the one
    multi-file full-post probe (D13) under strict T375. Untouched live
    uses never block the joint: uses the native leaves in place surface
    as `orphans` of the deletion hunk (`off-target` when anchored,
    `shadowed`/`second-hop`/`macro-hidden` per the member follow law,
    capping at `partial`); uses the native deletes or rewrites fail the
     probe and the joint declines. Unrelated same-named idents are not
     anchored uses and fire `verified`. Two deletions and
     attr+deletion combos never joint-fire (slice 5): distant deletions
     partition (the rename/attr joint still fires alone, deletions ride
     the ordinary remainder), adjacent multi-deletion hunks stay wholly
     ordinary via the shape-B trap. Two evolutions,
     two additions, an addition of another container, bare renames,
    and alias/const shapes never joint-fire (only
    multi-addition and cross-file-member joints stay out: a member in
    another file than its container cannot joint by the container law).
    Cross-kind members (WI-21 slice 4) veto the joint AND suppress the bare
    rename single — wholly ordinary with the stranded delta in declined
    provenance.
    Enum rename + variant-deletion joint (WI-21 slice 2): the enum half
    of the same ladder — an `enum` rename plus exactly one
    variant-deletion line (`- B,`, `- B(u32),`, `- B { x: u32 },`,
    `- B = 3,`, distant `@@ -N +M,0 @@` or adjacent to the def change
    as a (two-minus, one-plus) hunk) in the same file and section plus
    claimable uses raises to the same shared-rule two-hunk shape with
    the `enum Old {` context. Same untouched-use law as the struct
    half (a rewritten `Old::B` → `New::B` pair rides as a claimable
    rename use; the dangling `New::B` orphans at `partial`; untouched
    `Old::B` spellings fail the probe because the follow rewrites them).
  Joint `impl Trait for Type` (WI-3): a `trait` rename pair plus a
  `struct`/`enum`/`union` self-type rename pair ride as two authored
  follow hunks sharing one rule id (trait hunk first); the composed
  `impl OldTr for OldTy` header is never written — the two follows
  rewrite it at apply time, and raising emits that spelling for the
  single-file triple (SPEC D.4). Any third rename, any extra changed
  line, qualified trait paths, and `impl<…>` generics stay ordinary.

Never rewritten: field accesses (`s.old`), method idents (`s.old()` —
a different namespace), and `qself` paths (`<T as Tr>::Old`, declined).
Workspace search (WI-7a): the resolver searches every loaded `.rs`
file, not just the definition file. `applyrs` loads the search set
whenever the patch carries `# follow=uses` and lowers cross-file
rewrites even in files the patch never sections. Inside a git worktree
the set is tracked plus untracked-but-not-ignored `.rs` files (`git
ls-files --cached --others --exclude-standard`, minus `.git/`/`target/`/
`.history/` prefixes and symlinks), so ignored and generated files are
neither searched nor rewritten; outside a worktree it falls back to the
lexical walk (every repo `.rs` file outside those prefixes). One hunk
therefore renames the definition plus its uses repo-wide; the JSON
  report carries one entry per touched file under the same rule name
  (Section D.2) plus top-level `follow_files` (files in the search set)
  and `follow_source` (`"git"` or `"walk"`).

Unique-definition rule (precision-first, Section 2.6/D3): cross-file
follow rewrites only when exactly one definition of the old ident exists
workspace-wide. The common rename — one `Old`, used in N files — just
works. A second same-named definition anywhere makes every use ambiguous:
without `# fallback=syntax` the hunk vetoes with `RSMPL_SYMBOL_CONFLICT`
naming the first shadow (`path:line`); with it all use rewriting is
skipped and each shadow is listed in `follow_skipped_details`. Same-file
shadowing (a local binding, an enum variant) and opaque sites
(macro/attribute interiors — paths and values, `qself` paths, glob imports paired with a
same-name use) veto or skip exactly as before, now reported
`path:line`-qualified when the workspace holds several files;
single-file patches keep the historical `line N` wording.

Re-export hop (WI-7b): an explicit `pub use …::Old` in a non-definition
file that names the definition module is a proven one-hop re-export —
the definition's module path derived from its file (`src/a.rs` →
`a::Old`, `src/x/y.rs` → `x::y::Old`, `src/lib.rs` → `crate::Old`; one
leading `crate`/`super`/`self` is stripped before comparing, so
`pub use crate::a::Old`, `pub use super::a::Old`, and `pub use a::Old`
all name the same target). The re-export line itself plus uses spelled
through the re-exporter (`b::Old`, `crate::b::Old`, `use crate::b::Old`)
rewrite like any other use; several files re-exporting the same
definition (one hop each, single target) all fire. Anything else is
opaque: a re-export routed through another re-exporter is a second hop,
and any other prefix is off-target (it may name a foreign item) — both
veto with `RSMPL_SYMBOL_CONFLICT` naming the re-export (`path:line`),
or skip all use rewriting with per-site `follow_skipped_details` triples under
`# fallback=syntax` while the definition rename still applies. A
`pub use …::*` glob pointing away from the definition is likewise opaque,
but only when another file spells `Old` through that module (bare
`crate::Old` through a root glob counts too); globs forwarding the
definition module itself are benign, as are globs no through-use names.
Nested definitions must be re-exported with the full file-derived path
(`pub use crate::x::y::Old`); longer spellings that do not match stay
vetoed — without module-tree resolution a longer path may name a
different nested module.

Orphan evidence (WI-7c): every definition-only fn/item rename runs the
same syntactic index in report-only mode. The JSON rule entry gains
`orphans: [{file, line, reason, text}]` (one entry per stale-use line,
sorted by `file`/`line`, `text` is the trimmed source line) plus
`orphans_truncated` when more than 8 lines name the old ident (each
`text` truncated to 80 chars with a trailing `…` when cut) plus
`orphans_truncated_chars` counting cut texts (absent when zero). Opaque macro/attribute/`qself` lines
count as orphans; unparseable files are excluded (the tripwire above
owns them). `--check` stays green — the rename may be intentional —
and text mode prints one `orphans:` stderr line with each entry as
`file:line [reason]: text`. With `--verify build`
the first diagnostic names the first orphan `file:line` alongside the
`applied` flag. A `# follow=uses` hunk never carries `orphans` (its
uses rewrite; `follow_skipped_details` owns that signal).
WI-2a extends orphan evidence to `let`-binder renames: a general-statement `let old = E;` -> `let new = E;` hunk (same init, single ident binder, plus the single-leaf destructuring twin `let (a, old) = E;` -> `let (a, new) = E;` and struct/shorthand/slice/wrapper spellings) lists leftover same-name uses as `orphans` (`off-target`, uses past an inner same-name binder `shadowed`, macro and attribute interiors (paths, values, and list tokens) and inline `{old}` format captures `macro-hidden`) and caps at `reparse-only` (global `partial`) while leftovers exist; zero leftovers stays `instantiated`; only same-file uses at/after the binder statement end inside the binder's smallest enclosing block are orphans (pre-binder, cross-file, other-fn/other-block, and nested-fn same-name uses, including the binder's own init RHS, never dangle and are never orphans). WI-2b completes the same shapes in-block (plain and single-leaf destructuring alike; sequence-contained rename-once `let` renames (plain or single-leaf destructuring) at any single position complete post-window the same way, with authored init-substitution (`let b = old;` → `let b = new;`, same pat with a total old→new init swap strictly after the renamed position) completing later in-window uses by authoring while spelled-`old` in-window replays strand as `off-target` orphans through the narrowed sequence cover (def + follow + per-occurrence substitution spans, not the whole window; hole-bound subtrees contribute zero spans so smuggled shadow/macro olds surface; a pre-existing `new` binder in the renamed statement's own scope (same level or outer — the statement itself for Stmt single-`let` renames, the window for sequence rename-once) declines the follow arm and surfaces as `capture-risk` orphans)): unshadowed value uses of `old` after the binder inside its smallest enclosing block rewrite in the same hunk (labelled `follow`, the binder definition itself labelled `rename` like every other rename); inner `let old` / closure-param `old` shadows, nested-`fn` bodies, and macro and attribute interiors never rewrite (stay orphans); every other same-name binder (`for` / `if let` / `while let` pattern, match-arm, fn-param) governs only its construct scope (loop body, then-branch, arm guard+body, fn body — WI-13): uses inside that scope stay `shadowed` orphans while uses outside it still rewrite, so no unshadowed outer use strands as `off-target`; a post-binder same-name binder covered by no modelled scope (unmodellable syntax) still vetoes completion. Struct-literal shorthand in expression position expands member-style: `S { old }` rewrites to `S { old: new }` (field kept, value updated — the opposite direction from field renames, which emit `S { new: old }`); explicit `S { old: old }` rewrites only the value. WI-2c extends the same duty to closure-parameter and match-arm binder renames (general-expression `|old| Body` -> `|new| Body` and `match S { old => B, ... }` -> `match S { new => B, ... }` hunks with binder-only diffs, single ident binder): leftover uses *inside* the rewritten node — the only places that can dangle, i.e. hole-preserved body (closure) or guard/body (arm) regions — are `orphans` (`off-target`, uses past an inner same-name binder `shadowed`, in-scope macro and attribute interiors (paths, values, and list tokens) and inline `{old}` format captures `macro-hidden`) and cap at `reparse-only` (global `partial`) while any exist; the scrutinee, other arms, and all uses outside the node can never dangle and are never orphans; zero leftovers stays `instantiated`. WI-2c performs no completion (a future item owns block completion for these positions); closure qualifier deltas (`async`/`static`/`move`, lifetimes, constness, return type) veto detection, matching the engine's strict closure discipline. WI-2d extends the same duty to for-loop (`for old in E { ... }` -> `for new in E { ... }`), `if let` / `while let` (`if`/`while let old = E { ... }` -> `if`/`while let new = E { ... }`), and fn-parameter (same-name `fn` header, single-param `old: T` -> `new: T`) binder renames with binder-only diffs, single ident binder: leftover uses *inside* the scope that can dangle — loop body (`for`, `while let`), then-branch (`if let`), or fn body (fn-param) — are `orphans` (`off-target`, uses past an inner same-name binder `shadowed`, in-scope macro and attribute interiors (paths, values, and list tokens) and inline `{old}` format captures `macro-hidden`) and cap at `reparse-only` (global `partial`) while any exist; the iterator, scrutinee, `else` branch, signature, and all uses outside the scope can never dangle and are never orphans; zero leftovers stays `instantiated`/`postcondition`. WI-2d performs no completion (a future item owns block completion for these positions). Placeholder normalization in all binder detectors is whole-ident-token replacement on both sides, so the single-hole __RSMPL_E never fires inside __RSMPL_ELLIPSIS_N markers. WI-3 keys binder honesty on the compiled binder, never on surface spelling: any minus/plus pair that renames exactly one bound binder — `let`, closure-param, match-arm, `for` / `if let` / `while let`, fn-param, or a closure parameter nested in a `let` init (`let g = |old| E;` -> `let g = |sum| E;`, the F3 spelling) — enters the same orphan/completion/downgrade path (detections that cannot prove single-binder identity — init/type/body changes, qualifier deltas, multi-binder diffs, bare-`P` annotation add/drop — stay general templates with no binder privileges; a Stmt/Expr minus/plus pair whose hole-erased concrete bound-ident sets differ across `Pat` positions without binder identity — a plus-side binder minus never bound, or a dropped binder with no hole replay — never applies as a general template and fails at the C21 gate instead (SPEC 18.6; pure permutations and hole replays keep applying), because the general path files no orphans and would report `verified` with the uses stranded; sequences are exempt except a final-position plain-`let` rename, which completes and orphans like `let` (hetero fusion owns other binder-renaming runs there)). WI-3 completes every scope: the binder ident emits the `rename` edit and unshadowed in-scope value uses rewrite in-hunk (`follow`, with `S { old }` expanding to `S { old: new }`) — including field/method receivers headed by the binder (`old.field`, `old.method()`) and captures inside nested closures, which are binder uses, not anchor-table clients; shadowed, macro-hidden (macro and attribute interiors: paths, values, and list tokens), nested-fn, `qself`, and type-position uses stay orphans, never rewritten. The postcondition probe adjusts recorded bindings for in-scope holes (so completion re-matches), leaving out-of-scope holes byte-identical.

Skip and orphan reason vocabulary (WI-2): every structured skip and
every orphan carries a machine `reason`, one of `shadowed` (another
container defines the same name, so no site is provable), `glob-reexport`
(a `use …::*` glob may import a different `old`), `second-hop` (a
re-export chain passes through a second re-exporter), `macro-hidden`
(macro/attribute/`qself` interiors the resolver cannot attribute, or
an unsearchable workspace file), `off-target` (names `old` but sits
outside any rewritten hunk: a re-export that does not point at our
definition, or a use left behind by a definition-only rename),
`chain-depth` (WI-11: a method chain deeper than the bounded-two cap),
`chain-unproven` (WI-11: a chain step naming no proven constructor or
stated return type), `ambiguous-bound` (WI-11: a generic-bound receiver
with two or more candidate bounds), `capture-risk` (a pre-existing `new` binder in a sequence- or single-`let`-renamed binder's own scope — rewritten uses would resolve under a shadowing twin). JSON
`follow_skipped_details: [{file, line, reason}]` carries the skip signal alone:
the historical `; `-joined `follow_skipped` prose string was removed in
report schema `version: 4` (WI-20 owns the string's
removal); text mode prints one `follow-skipped: {file}:{line}:{reason}`
stderr line per skip. Whole-file skips (unparseable workspace files)
report `line: 0` — every real line is 1-based, so 0 unambiguously means
whole-file.

Out of scope in v0.1: non-`.rs` files are never searched; symlinked
files/dirs are skipped; a workspace `.rs` file that does not parse trips a
fail-loud tripwire when its text names the ident outside strings/comments
(`RSMPL_SYMBOL_CONFLICT` naming the file, or a `follow_skipped_details` triple with
`# fallback=syntax`); multi-definition workspaces never attribute uses by
module path (WI-7b owns only re-export prefix matching, never use
 attribution); `diffrs` raises item intents as `item_intent` (SPEC D.4:
 one definition hunk with `# follow=uses` when the cross-file
 scratch-apply probe proves the group's changed lines, else
 definition-only per-file hunks with uses ordinary — never an unproven
 follow), member intents as `member_rename`, and `mod` + file-rename
 intents as `mod_rename` (same probe posture, file/section-pure
 partial). WI-7 grouping owns the shared-intent spelling for all other
 same-delta hunks.

Fail loud (Section 2.6): a second same-named definition, a shadowing
local binding or enum variant, or a macro/attribute mention the resolver
cannot attribute vetoes the whole hunk with `RSMPL_SYMBOL_CONFLICT`
naming the site — unless the hunk also carries `# fallback=syntax`, in
which case unprovable sites are left untouched and listed in the WI-2
`follow_skipped_details` triple while provable sites still rewrite (a shadowed
definition or an ambiguous re-export hop skips *all* use rewriting, since
every use is then ambiguous;
a glob skips only that file's uses).
On any other shape `# follow=uses` is `RSMPL_PATTERN_CONFLICT` (Section
18.6: other shapes keep the follow-up hunks above).

> **v0.1 implementation note:** `const fn` shapes keep routing to the function classifier (the keyword in name position falls through); `impl!(...)` macro shapes keep routing to the macro classifier (a `!` after the kind falls through). Item-rename hunks reject `# returns=none` (function-only). `auto trait` headers and raw-identifier edge cases beyond `` `r#name` `` are not supported; use an ordinary hunk.

#### Member renames (WI-9a)

Renaming a struct field or an enum variant is rename-only like 14.8, but the name lives inside a container, so the author writes the container as a context header plus a member pair (the impl-context spelling family, Section 29.9). The header is a filter in every scope (Section 13.4 posture): only the named container matches.

```diff
  struct S {
- old: u32
+ new: u32
```

```diff
  enum E {
- Old
+ New
```

Rules:

- A field pair is `[pub…] old : Type` / `[pub…] new : Type`: names differ, visibility prefixes agree (whitespace-insensitive), and both types render equal (`quote!`-normalized, so spacing never matters). Only the field ident span is replaced; container, visibility, and type are preserved. The target field's type must render equal to the annotation — a wrong annotation skips the site (`RSMPL_MATCH_CONFLICT`), never rewrites under a lie. Named fields only: unit/tuple structs and unions never match (ordinary hunks).
- A variant pair is bare `Old` / `New`: names differ, only the variant ident span is replaced; discriminants and tuple/struct fields are preserved. Unit, tuple, and struct variants all rename.
- Classification is shape-first, context-second: both sides must parse as the same member-pair shape before any context line is consulted, so ordinary hunks merely sitting inside a struct/enum body never route here. A field pair without a `struct S {` context line fails fast naming the missing header; colon-carrying non-member shapes (`use crate::old`, `impl std::fmt::Display`, `struct Old { x: i32 }`, call rewrites) fall through to their own classifiers untouched. A bare-ident pair without an `enum E {` context line falls through unchanged (never a member rename without its container).
- Scope modes, attribute policies, and `allow`/`optional` counting mirror item renames (anchor on the member ident line, envelope on the container span).

`# follow=uses` on a member rename rewrites anchored use sites workspace-wide (WI-14 anchor classes, strongest first; sibling same-named definitions never veto — the definition hunk always applies, unproven sites are `shadowed`/`off-target` evidence):

- Fields: literal-path sites are proven by the path — struct-literal labels (`S { old: v }`) and struct-pattern labels (`S { old: p }`) whose path names `S` (last segment equality, plus `Self`-spelled paths inside an `impl` of `S`). Shorthand labels expand keeping the local binding: `S { old }` rewrites to `S { new: old }`. `.old` accesses are proven only by a receiver anchor: fn-signature params (`fn f(a: A)` proves `a.old`), `let x: A` annotations, `let x = S { … }` struct-literal inits, and `self` inside `impl S` (all flow-ordered per fn body; a later `let x: B` re-anchors). The closed wrapper set derefs one step: `Box<S>`/`Rc<S>`/`Arc<S>` (single-segment spelling unless shadowed by a local def/use, or multi-segment on a `std`/`alloc`/`core` prefix) anchors to `S`, composing with `&`/`&mut`; double-deref stays off-target. A workspace `impl Deref for C { type Target = T; }` (any file in the repo tree; conflicting `T` for one `C` anchors nothing; `impl foo::Deref` with a non-std multi-segment trait path never counts) anchors `C` to the outer tail of `T` (one step; `Box<C>` still unwraps to `C`, never chains to `T`; `Target = Self` is a no-op). `-> A` returns never anchor; call/literal bases (`make_a().old`) never claim. Unproven accesses are `shadowed` (another container defines the name) or `off-target` evidence — never a run failure. Method idents named `old` and tuple `.0` accesses never match and never veto (different namespaces).
- Variants: expression/pattern paths spelling `E::Old` (any leading prefix, plus `Self::Old` inside an `impl` of `E`) and `use …::E::Old` import segments are proven (literal-path). Bare `Old` rewrites only with a `use …::E::*` glob anchor naming `E` in the same file; otherwise it is `second-hop` evidence (never a veto). `qself` paths and macro interiors mentioning the member name are opaque like scalar renames (veto without fallback, skip + report with it).
- Orphan evidence mirrors scalar renames: a definition-only member rename lists anchored use lines as `orphans` (`off-target` for proven, `shadowed`/`second-hop` for unproven, `macro-hidden` for interiors) while `--check` stays green; a `# follow=uses` hunk never carries `orphans` (`follow_skipped_details` owns that signal, each with its WI-2 `reason`).

Out of scope in v0.1: tuple-struct positional fields, locals/params, unions, cross-crate resolution, multi-definition sections, and any use attribution beyond the syntactic shapes above (`diffrs` raises single-file def+uses intents via `member_rename` per section and cross-file def+uses intents via the `member_rename` whole-patch pre-pass, SPEC D.4; everything else stays ordinary).

#### Definition RHS evolution (WI-11a/WI-11b)

A definition's *value* side — the `= …` of a type alias, `const`, or `static` — evolves without renaming anything. The author writes the definition line with a concrete preserved name and a template RHS; only the RHS span is replaced, everything else (visibility, generics, `where` clauses, attributes, the trailing `;`) is preserved byte-for-byte:

```diff
@@ -1,1 @@ rule=evolve_alias
# lang=rust
- type A = Vec<T>;
+ type A = Box<[T]>;
```

```diff
@@ -1,1 @@ rule=evolve_const
# lang=rust
- const MAX: i32 = E;
+ const MAX: i32 = E + 1;
```

Member-context shapes reuse the member-rename header+pair spelling (the
container header is the disambiguator: without `struct S {` / `enum E {`
context, call-shaped pairs belong to the call classifier):

```diff
@@ -1,3 +1,3 @@ rule=evolve_field
  struct S {
- old: Vec<T>
+ old: Box<[T]>
```

```diff
@@ -1,3 +1,3 @@ rule=evolve_payload
  enum E {
- Old(Vec<T>)
+ Old(Box<[T]>)
```

```diff
@@ -1,3 +1,3 @@ rule=evolve_disc
  enum E {
- Old = E
+ Old = E + 1
```

Rules:

- Type-alias evolution is `type NAME = RHS;` with a concrete `NAME` on both sides and a type template for the RHS (`T` holes ride the existing diet/`# type T` inference, SPEC 15.8). Generics are preserved from the target, never authored (`type A<T> = …` is a pattern error). Visibility prefixes must agree; a vis change is a WI-9 header evolution, not this shape.
- Const/static evolution is `const NAME: TY = EXPR;` (or `static [mut] NAME: TY = EXPR;`) with concrete `NAME`, token-equal `TY` on both sides, and a general-expr template for the value (`E` holes ride diet/`# expr E` inference). `static mut` must agree on both sides; `const` vs `static` never combine. A mismatched ascribed type at a site skips the site (subset semantics — never rewrites under a lie); a mismatched `mut` skips likewise.
- Field-type evolution is `struct S {` context + `old: OLD_TY` / `old: NEW_TY` with concrete preserved `old` and a type template on each side (diet `T` infers from the RHS region). Same-type pairs belong to the field-rename classifier (a different name with the same type is a rename, claimed via fallthrough); different names with different types never combine (two hunks, or an ordinary hunk).
- Variant-payload evolution is `enum E {` context + one of three same-shape pairs with concrete preserved variant name: tuple payloads `Old(A, …)` / `Old(B, …)` (same arity, per-position type templates sharing one binding set under SPEC 15.7), struct payloads `Old { f: A, … }` / `Old { f: B, … }` (same field names in order, per-field type templates), or discriminants `Old = D0` / `Old = D1` (a general-expr template). Payload and discriminant never evolve together (one hunk per region, or an ordinary hunk). A bare `Old` / `New` pair stays a variant rename; call-shaped pairs without the header stay calls.
- A name-changing "evolution" fails fast: same RHS on both sides points at the bare `<kind> <Name>` rename pair; differing RHS on both sides never combines rename + evolution (write a rename hunk plus an evolution hunk, or an ordinary hunk).
- Classification is shape-first: sides without `=` (bare `type Old`) fall through to the item-rename classifier untouched; `const { … }` block expressions are never definitions. Member pairs without their container header fall through (field pairs to the member-rename missing-header error; call-shaped and discriminant pairs to the call/general classifiers). Leading `#[...]`/doc lines are the member's own attributes (SPEC 14.6 mirror laws: plus-side additions and differing sets fail fast; matching sites carry them).
- `# follow=uses` is rejected on evolution kinds (C17): evolving a type renames nothing, and silently following would guess at conversion edits the patch never stated. Member addition kinds (`FieldAddition`, `VariantAddition`, `ArmAddition`) reject it likewise (an insertion renames nothing). Verification is the WI-1.5 plus re-match at each reported span with per-site binding equality (`instantiated` verdict) for evolutions; pure insertions and member additions verify as `postcondition` via their structural probes (presence plus standalone re-parse / container membership), else `reparse-only`.
- Out of scope in v0.1: generic-param/`where` evolution on aliases and fields, and value conversion at use sites.

Member-addition authoring (WI-12b slice 1) reuses the member-evolution
header+member spelling with an empty minus member set: the container
header (`struct S {` / `enum E {`) is context and the single `+`
member line is the point insertion. Match-arm addition (WI-12d) is the
same spelling over a `match <scrut> {` context line: empty minus, one
`+` arm line (`syn::Arm`-validated, never a metavariable hole).

```diff
@@ -2,0 +3 @@ rule=add_field
  struct S {
+ b: bool,
```

```diff
@@ -2,0 +3 @@ rule=add_variant
  enum E {
+ C(i32),
```

```diff
@@ -6,1 @@ rule=add_arm
  match e {
+ E::C => 3,
```

Rules: the minus side must be empty (any minus member belongs to
evolution/rename/ordinary); the plus side must be exactly one
field/variant line (two-plus lines stay ordinary); the container
header must be present (member lines without it fall through to the
same classifiers as evolution pairs). Ranged hunks cite the
insertion point (`-N,0` inserting after line N, which must lie
strictly inside the container); `# sweep` inserts before the closing
brace of every matching container lacking the member (already-present
members skip the site). An already-present member is 0 matches, never
an overwrite. Attr lines on the plus member ride alongside (SPEC 14.6
attach discipline); `# follow=uses` is rejected (C17). Match-arm
rules: the arm path names the variant by last segment with an
optional single-segment enum prefix (`E::C` or bare `C`; longer paths
need symbol resolution and decline, a lowercase bare ident is a
binding and declines); the context scrutinee must equal the target
match's scrutinee token-for-token; insertion lands before the closing
brace of the innermost spanning match (a brace sharing its line with
other code vetoes); arm additions are ranged-only in v1 (a sweep
would plant the arm into matches on other enums and fails fast with a
template).

Associated-item rename (WI-13 slice 1) renames a `type` or `const`
member inside a `trait` or `impl` block. The author writes the block
header as a context line plus a bare assoc pair (the member-rename
spelling family):

```diff
  trait Tr {
- type Item;
+ type Output;
```

```diff
  impl S {
- type Item = i32;
+ type Output = i32;
```

```diff
  impl S {
- const MAX: i32 = 10;
+ const LIMIT: i32 = 10;
```

Rules (rename): names differ, everything else token-equal (generics,
bounds, default types, ascribed types, values — preserved). Only the
member name ident span is replaced. Classification is shape-first,
context-second (checked before the method-context router: `const …`
lines satisfy the fn lookahead, and a `const fn` method header
explicitly falls through to methods; evolution is checked before
rename — same-name RHS deltas route to evolution, name deltas to
rename). A pair without a `trait T {` / `impl … {` context line falls
through untouched (free `type`/`const` pairs stay item renames /
definition evolutions). Rename verification is span-local ident
equality at the reported spans (`instantiated`).

`# follow=uses` on an assoc rename (WI-27, definition set completed by
WI-1) rewrites the container-proven use tails plus, for a trait-header
rename, every trait-proven `impl <Trait> for <Self>` definition of the
same kind and name (`type Item = …` / `const LIMIT … = …` inside an
`impl Tr for S` block whose trait tail names the renamed trait) —
each definition ident rewritten and labelled `rename` in `span_labels`
(use tails stay `follow`; `matches` still counts definitions). Inherent
impls (`impl S { … }` with no trait) never claim: they are a different
item, so an inherent same-kind `old` definition is left untouched and
reported as `shadowed` evidence. An impl header that does not
textually name the trait (glob/second-hop) yields no definition claim
(skip, never veto, never whole-run failure). Use tails rewrite in
expression, type, and pattern positions:
`Self::Old` inside a domain impl block (a header-matching block, or an
`impl Tr for S` block when the renamed trait is `Tr`; `Self::Old`
inside the matching `trait Tr` block itself likewise), `S::Old` /
`Tr::Old` naming the domain self type / trait (any leading prefix is
preserved; trailing tails such as `S::Old::default()` keep their tail),
`<S as Tr>::Old` naming the trait explicitly, and `use C::Old` leaves
under the same container rule (`use Self::Old` names the module, never
the type). Only the trailing `Old` ident span is replaced. A second
trait — or, for `const`, an inherent impl — defining the same assoc
name vetoes only its own blocks: sites enclosed in (or naming) those
blocks are skipped with reason `shadowed` while provable sites still
rewrite (never a whole-run failure, with or without
`# fallback=syntax`). Macro-token / attribute interiors mentioning
`old` are opaque: veto without fallback, skip with `macro-hidden` under
it (scalar/member parity). Bare `Old` is not assoc syntax and is
ignored outright. Without follow, the same claim walk plus the WI-1
definition set reports every left-behind site as an orphan
(`off-target`, macro interiors `macro-hidden`) — including leftover
trait-proven `impl` definitions. Follow-only file extras are
`reparse-only` by construction; the definition probe covers the
rename.

`# follow=uses` on a trait-header method rename (WI-3, `trait Tr {`
context + `fn old` → `fn new`) rewrites the trait declaration plus
every trait-proven `impl <Trait> for <Self>` definition of the same
name — each impl `fn` ident rewritten and labelled `rename` in
`span_labels` (`matches` counts the trait declaration) — plus anchored
call sites (WI-21): `recv.old()` where the WI-14 anchor order proves
the receiver names a domain self type (fn-signature params, `let x: C`
annotations, `let x = S { … }` literal inits, `self` inside a domain
`impl S` or inside the defining `trait Tr` default body — WI-10 lets
`Box<S>`/`Rc<S>`/`Arc<S>` receivers anchor to `S` through one deref
step under the same closed-set/shadowing law, plus workspace
`impl Deref for C` targets anchoring `C` to `T`), and `S::old`
tails where `S` names a domain self type (or the trait itself), plus
`<S as Tr>::old` qself tails naming the trait. WI-11 grows the anchor
order same-file only (D8): `C::ctor(…).old()` where `ctor` is
`new`/`make`/`default` and the same file contains `impl C` with that
constructor returning `C`/`Self`; `base.m(…).old()` where `base` is
anchored and the same file states `m` returning the domain type on the
base type (bounded two hops — each hop proved, a third hop skips
`chain-depth`; a deref step costs no hop); and `t.old()`
where `t: T` with exactly one bound naming the trait (`T: Tr`, inline
or `where`; two or more bounds skip `ambiguous-bound`); and WI-10
slice 2 grows the one-hop arm workspace-wide: `base.m(…).old()`
where `m` is a workspace trait method returning `Self` proves the
anchored base type (exactly one such trait workspace-wide — twins stay
`chain-unproven`), and `base.clone().old()` proves the anchored base
type by the closed `clone`-shape. Bare `old` is
not method syntax (ignored); literal/composite bases that match no
chain shape (`make().old()`) never claim; unrelated `X::old` tails are
ignored outright. Unproven method receivers are `shadowed` (another
item defines `old`), `off-target`, or the WI-11 chain reasons
(`chain-depth`, `chain-unproven`, `ambiguous-bound`) evidence — never
a veto. Inherent `impl S { fn old }` and other-trait
`impl <Other> for _ { fn old }` defs never claim a trait-header rename
(different items → `shadowed` evidence). Macro-token / attribute
interiors (paths and values) mentioning `old` are opaque: veto without
fallback, skip with `macro-hidden` under it (scalar/member parity).
Without follow, leftover
trait-proven impl defs and anchored call sites report as orphans
(`off-target`, unproven `shadowed`, macro/attribute `macro-hidden`).

`# follow=uses` on a bare-inherent-`impl` method rename (WI-6, `impl S {`
context + `fn old` → `fn new`) rewrites every inherent `impl S`
definition of the same name — each `fn` ident rewritten and labelled
`rename` in `span_labels` (`matches` counts the definition-hunk block) —
plus anchored call sites by the same anchor order over the single-type
domain (`S`): `recv.old()` with a proven `S` receiver (WI-10: `Box<S>`/
`Rc<S>`/`Arc<S>` receivers anchor through one deref step under the
closed-set/shadowing law, plus workspace `impl Deref for C` targets
anchoring `C` to `T`), `self` inside an
`impl S` block, `S::old` tails, and `<S as _>::old` qself tails naming
the self type, plus the WI-11 same-file chain shapes (constructor calls
and bounded-two method-return chains, including workspace trait-`Self`
returns and the closed `clone`-shape; generic bounds never prove a concrete
self type and stay `off-target`). `trait` declarations, `impl <Trait> for _` definitions,
and other-type inherent `impl U { fn old }` defs never claim (different
items → `shadowed` evidence); unproven receivers are `shadowed`,
`off-target`, or the WI-11 chain reasons, never a veto. Macro-token /
attribute interiors (paths and values) mentioning `old` are opaque:
veto without fallback, skip with `macro-hidden` under it
(scalar/member parity). Without follow, leftover domain defs and
anchored calls report as orphans with the same reasons. The definition
set is every inherent `impl S` block of the domain (last-segment
equality, D8). `impl T for S` headers name a trait-impl block, not a
renameable method owner, and same-name evolutions rename nothing: both
keep the C17 rejection.

Associated-item evolution (WI-26) keeps the name and rewrites only
the RHS/default span under the same header: `type Item = Vec<T>;` →
`type Item = Box<[T]>;` (RHS is a type template, `T` holes ride diet
inference; generics/bounds preserved token-equal) and
`const MAX: i32 = E;` → `const MAX: i32 = E + 1;` (ascribed type
preserved token-equal, value a general-expr template). Both sides
must carry an RHS/default (a bare decl carries nothing to evolve);
one-sided defaults and preserved-region mismatches fail loud.
`# follow=uses` is rejected (C17, definition only). Verification is
structural plus re-match at the reported spans (`instantiated`).

Raising (detector `assoc_rename`, WI-28, single-file): K ≥ 1
definition pairs in `trait`/`impl` blocks (one per block, same kind
and ident delta, per-pair suffixes preserved) plus use-site lines
sharing the delta through a `::Old` tail compress to one hunk per
enclosing block (verbatim headers from the pre-image, shared rule)
with `# follow=uses` on the first hunk; the scratch-apply
line-equality probe is the totality (the authored emission applied
to the pre-image must reproduce exactly the native plus lines at
the changed sites with every other line byte-identical). Bare-`Old`
uses (no `::` tail), two-trait same-name classes (two intents, never
grouped), hetero kinds, and multi-def blocks decline to ordinary. A
definition-only hunk whose intent sibling (same rule, any file)
carries the follow reports no orphans of its own (the follow hunk
   owns the use set). Cross-file intents raise via the WI-12 whole-patch pre-pass (one section per def file, shared rule, follow on the document-first hunk; off-diff `::Old` uses veto).
Out of scope in v1: inherent-`impl` domain distinctions beyond header
equality.

### 14.9 Use-path prefix renames

Renaming a module (Section 14.8) orphans its imports. One semantic hunk
moves them back: the author writes a bare `use <prefix>` pair and the
engine rewrites the matched leading-segment span of every `use` item in
range, preserving the target's tail verbatim:

```diff
diff --git src/lib.rs
@@ -1,7 @@ rule=move_imports
# allow=many
- use crate::old
+ use crate::new
```

This rewrites `use crate::old::Thing;`, `use crate::old::{A, B};`,
`use crate::old::*;`, `use crate::old::Widget as W;`, and
`pub use crate::old::Reexport;` by replacing only the `crate::old` span —
tails, visibility, attributes, and formatting are preserved byte-for-byte
(the same fast path as a name-only item rename).

Rules:

- Both sides must be bare `use <path>` prefixes with plain `::`-separated
  identifier segments (optional trailing `;`, optional leading `pub…`,
  optional leading `::`, all normalized away). Anything else in the
  pattern — braces, globs, `as`, `!`, `...`, `$Meta`, backticks, or a
  declared-metavariable segment — is a pattern error; targets may freely
  contain groups/globs/`as` tails, which are preserved — including members
  sitting directly inside a group at the matched position (see below).
   The seven pattern-side rejections are engine-verbatim (all exit 2; all carry `rule:`/`file:`; text omits the null `range:`/`suggestion:` lines, JSON carries explicit `range: null` / `suggestion: null`; `{Name}` is the offending segment spelling):
   ```text
   RSMPL_PATTERN_CONFLICT
   rule: r
   file: src/lib.rs
   reason: grouped (`::{...}`) and glob (`::*`) tails are not supported in the `use` *pattern* in v1 (write the bare shared prefix; target items may still contain groups/globs)
   ```
   ```text
   RSMPL_PATTERN_CONFLICT
   rule: r
   file: src/lib.rs
   reason: `as` renames are not supported in the `use` *pattern* in v1 (write the bare prefix; target `as` tails are preserved)
   ```
   ```text
   RSMPL_PATTERN_CONFLICT
   rule: r
   file: src/lib.rs
   reason: v1 `use`-path patterns do not support `...` (write the concrete prefix path)
   ```
   ```text
   RSMPL_PATTERN_CONFLICT
   rule: r
   file: src/lib.rs
   reason: `$Meta` abstracted names are not supported for `use`-path renames in v1 (write concrete paths)
   ```
   ```text
   RSMPL_PATTERN_CONFLICT
   rule: r
   file: src/lib.rs
   reason: backtick literal quoting is not supported in `use`-path patterns in v1 (write the concrete path)
   ```
   ```text
   RSMPL_PATTERN_CONFLICT
   rule: r
   file: src/lib.rs
   reason: metavariable `{Name}` is not supported in `use`-path patterns in v1 (write the concrete path)
   ```
   ```text
   RSMPL_PATTERN_CONFLICT
   rule: r
   file: src/lib.rs
   reason: `use!` is a macro invocation, not a `use`-path rename (remove the `!` or use a macro pattern)
   ```
  The tail family is engine-verbatim too (same envelope + nulls; `{body}`/`{seg}` echo the offending spelling):
   ```text
   RSMPL_PATTERN_CONFLICT
   rule: r
   file: src/lib.rs
   reason: mirrored attributes are not supported in `use`-path patterns in v1 (docs are preserved by default; drop the attribute lines or use an ordinary hunk)
   ```
   ```text
   RSMPL_PATTERN_CONFLICT
   rule: r
   file: src/lib.rs
   reason: invalid `use` path `crate::old::::bad` (expected `::`-separated identifiers)
   ```
   ```text
   RSMPL_PATTERN_CONFLICT
   rule: r
   file: src/lib.rs
   reason: invalid `use` path segment `Old<M>` (expected an identifier)
   ```
   ```text
   RSMPL_PATTERN_CONFLICT
   rule: r
   file: src/lib.rs
   reason: missing path after `use` (write a bare `use <path>` prefix)
   ```
- Matching is segment-wise on the target's leading path segments: minus
  `crate::old` matches `crate::old::Thing` but never `crate::old2::Thing`
  (never a string prefix). Identical minus/plus prefixes are a pattern error, reported verbatim:
  ```text
  `use`-path rename minus and plus prefixes are identical (`crate::old`); write the new path on the `+` side
  ```
  (rule + file lines; text omits the null range/suggestion lines, JSON carries explicit nulls; suggestion is null).
  A pair where only one side is a bare `use` prefix is a pattern error too, reported verbatim (same envelope + nulls; suggestion is null throughout):
  ```text
  `use`-path rename plus side must be a bare `use <path>` prefix matching the minus side
  ```
  ```text
  `use`-path rename minus side must be a bare `use <path>` prefix matching the plus side
  ```
  The other side must be a different classifier's shape — probed minus `use crate::old` / plus `foo(E)` and vice versa with a call-shaped other side. A `- use …` / `+ fn …(...)` pair never reaches this gate (the `...` fires the `...` fence first); identical bare prefixes fire the identical fence above first. The item pair lives in Section 14.8.
- Group members at the matched position rewrite too: the walk descends
  into `{...}` (at any depth, including a root group) and matches each
  member from the member start, so `use crate::{old, other as o}` becomes
  `use crate::{new, other as o}` and `use crate::{old::Thing, other}`
  becomes `use crate::{new::Thing, other}` (`as` renames keep their alias;
  only the leading ident span is replaced). The member replacement is the
  plus suffix aligned to the matched minus suffix, which requires the
  leading segments to be literally shared — otherwise the member is
  skipped, never guessed: a divergent plus prefix skips grouped members
  (non-grouped sites still rewrite), a shrinking plus (empty replacement
  text) skips, and `crate::{x::old}` never matches minus `crate::old`
   (members match from member start only — no suffix overreach). Genuinely
   ambiguous member shapes are ordinary-hunk territory. Raising
   cross-reference (no law change): the `fn` item-intent raiser (SPEC D.4)
   claims exactly these tail-or-direct-group-member `use` deltas
   (`use crate::a::old;`, `use crate::a::{old, Other};`,
   `use crate::a::old as f;`, `pub use` forms) as part of a rename intent;
   suffix members like `crate::{x::old}` never join the intent, mirroring
   the no-suffix-overreach law above.
- Matching reuses the item scope modes (`scope=contained/anchor/overlap/
  exact`, anchor on the prefix-start line), attribute policies
  (`# attrs=...`), and `allow`/`optional` count validation. A file with
  several matching imports needs `# allow=many` (default `one` fails on
  the second match, like any multi-match hunk). Only file- and
  inline-`mod`-level `use` items are collected; `use` declarations nested
  inside function bodies are not matched in v0.1. Each match emits one
  prefix-span edit, so overlapping-match rules are trivially satisfied.
- Use-regroup merge (no new directives): N≥2 minus `use <prefix>::<Tail>;`
  lines plus one plus `use <prefix>::{<members>};` line stating their
  grouped form merge into one grouped item:
```diff
diff --git src/lib.rs
@@ -1,2 @@ rule=merge_uses
- use a::b::X;
- use a::b::Y;
+ use a::b::{X, Y};
```
  Merge-or-split, order-preserving (the group lists the tails in minus
  order — the tool never manufactures order), plain `Name` members
  only, uniform visibility across minus and plus lines
  (a regroup never changes visibility), no attribute lines on either
  side, no duplicate tails, and a non-empty shared prefix with every
  plain line exactly at the grouping depth (`<prefix>::<Tail>`).
  `as`/`glob`/nested-group members, mixed visibility or mixed leading
  `::`, deeper members, and wrong-arity shapes are pattern errors
  (same envelope + nulls as the tail family above; each names its
  tail or law). The merge engine replaces the first-in-file member
  line with the verbatim plus line (keeping that line's indent) and
  deletes the other member lines whole (attribute lines above a
  deleted item go with it); only single-line `use` items match and
  each member must be its own line. Default `one` merges exactly one
  member set (a second identical set fails loud); `# allow=many`
  merges every set. The split engine replaces the grouped item line
  with the N verbatim plus lines (each with the matched line's
  indent); default `one` splits exactly one grouped item (a second
  identical group fails loud); `# allow=many` splits every identical
  group. Raising (detectors `use_regroup` for merge, `use_split` for
  split): the per-hunk fallback claims the shape only for adjacent
  minus lines (merge; split needs only its single minus) under the
  precision guard (`semantic <= ordinary + 16` — a regroup never wins
  strict T375 since the `rule=` tag costs bytes over the ordinary
  header) with the D13 scratch-apply probe as totality (exactly one
  occurrence set in the pre-image plus byte-identical reproduction of
  the native post-tree); without a pre-image the shape stays ordinary
  (a line-deleting raise without proof is data-loss risk, so the
  section-visible legacy allow does not apply here). Non-adjacent
  merge minus lines stay ordinary (the native plus lands at the
  last-minus position while the engine merges at the first — the
  adjacency gate names the reason instead of a bare probe miss).
  Merge and split are one equivalence: `merge(split(x)) == x` and
  `split(merge(x)) == x` canonically (same prefix, same members in
  the same order, same visibility).
```diff
diff --git src/lib.rs
@@ -1,1 @@ rule=split_uses
- use a::b::{X, Y};
+ use a::b::X;
+ use a::b::Y;
```
- No `# root=` is needed: a `use`-led minus/plus pair routes
  automatically (a `use_foo(E)` call keeps routing to the call matcher by
  the word-boundary rule). `# repeat` and `# returns=none` are rejected
  on `use` hunks with the Section 17.8 other-shape reason and the Section 14.2 function-only reason, respectively (both engine-verbatim there).

Composition (no new shapes needed): call-site path prefixes already
rewrite with the call matcher (`- old::foo(E)` / `+ new::foo(E)`), and
type paths with the general type matcher (`- old::Thing` /
`+ new::Thing`). Trait-side paths in impl headers need no extra shape: the
bare `impl Old` / `impl New` pair (Section 14.8) rewrites them.

---

## 15. Metavariables

Metavariables bind Rust syntax or semantic objects.

RSmPL deliberately allows compact metavariable use such as `E`, `T`, and `I`, because that is token-efficient for LLM-generated patches. This creates a possible ambiguity when the source code itself contains an identifier with the same spelling as a declared metavariable. This section defines the resolution rule and the escape syntax.

### 15.1 Supported metavariable kinds

Implemented in v0.1 (fail fast on anything else):

| Kind | Matches |
|---|---|
| `symbol` | resolved semantic symbol (via `# symbol Name=path`, not `# symbol Name`) |
| `expr` | expression |
| `stmt` | accepted as a structural placeholder (e.g. the unused `# stmt S` in `let` examples); not a distinct statement binder |
| `item` | item, including its attached attribute/doc-comment envelope unless overridden by `# attrs=...` |
| `type` | type |
| `pat` | pattern: a whole-pattern placeholder, a concrete pattern, WI-3 structural composites with holes in bindable leaf positions (`Some(P)`, `(P, Q)`, `S { f: P }`, `&P`, `[P]`, `(P)`), or `P: T` parts — in `let`, match-arm, `for`, closure-param, and `let`-expression positions (Section 18.6 C8). A composite's path that is itself a whole hole (e.g. `# type S` over `S { f: P }`) binds the whole path; paths with holes anywhere else stay concrete |
| `generic_params` | generic parameter list |
| `where_clause` | where clause |
| `visibility` | visibility marker such as absent, `pub`, `pub(crate)` |
| `qualifier` | one function qualifier |
| `qualifier_list` | zero or more function qualifiers |
| `block` | block expression or function body (structural) |
| `macro` | macro path/name |
| `token_tree` | macro token tree or token sequence |
| `ident` | name hole: a function-header name (WI-5; binds the site ident, replays it on the plus side) or a bare-pair item name (WI-10b; binds per site, replays byte-identical for attr-only evolution; Section 14.1.2) |

Rejected in v0.1 with `RSMPL_PATTERN_CONFLICT`: `identifier`, `path`,
`lifetime`, `field`, `attributes`. These parse as declarations but have
no kind-specific engine binding — an agent authoring `# lifetime L`
pays tokens for a lifetime match that can never happen (Section 2.6).
(`ident` is the implemented name-hole kind above; `identifier` stays
rejected — write `# ident`, Section 15.8.)
`# attributes A` is rejected the same way: v0.1 has no attribute-list
abstraction, so write the `#[...]` lines literally on both sides to
constrain them, or omit them (attributes are preserved by default,
Section 14.6). `stmt`, `block`, `macro`, and `token_tree` are accepted
as structural placeholders (kind unchecked), not as distinct
kind-checked binders.

Engine-verbatim (all exit 2; all carry `rule:`/`file:`; text omits the
null `range:`/`suggestion:` lines, JSON carries explicit `range: null` /
`suggestion: null`). The first four kinds share one template (`<kind>`
and `<Name>` are the offending spelling, backticked as a whole — here
`# identifier I`):

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: a.rs
reason: `# identifier I` is not supported in v0.1 (see SPEC 15.1 for the implemented kinds)
```

`# attributes A` has its own reason (`A` is the offending name):

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: a.rs
reason: `# attributes A` is not supported in v0.1 (attribute lists cannot be abstracted; write the `#[...]` lines literally on both sides to constrain them, or omit them — attributes are preserved by default, SPEC 14.6)
```

### 15.2 Naming

Metavariable names should be uppercase or UpperCamelCase to distinguish them from ordinary Rust identifiers.

Recommended:

```text
E
Receiver
Arg
Old
New
I
T
P
```

Not recommended:

```text
x
foo
bar
```

Name validation is case-insensitive: any leading letter-or-`_` followed
by alphanumeric-or-`_` characters is accepted (lowercase included),
with no rejection path and no configuration knob in v0.1. Uppercase
stays a style recommendation so patterns read apart from ordinary Rust
identifiers. No warning is emitted for lowercase names in v0.1.

### 15.3 Declaration examples

```diff
# expr E
# type T
# item I
# pat P
```

### 15.4 Metavariable use and literal shadowing

A declared metavariable name shadows literal Rust syntax of the same spelling in compatible syntactic positions.

Example:

```diff
@@ -50,8 @@ rule=box_type
# type E
- Result<T, E>
+ Result<T, Box<E>>
```

In this hunk, `E` is a type metavariable. The pattern does **not** require a literal type parameter named `E`; it binds whatever type appears in that position.

If the pattern must match the literal Rust identifier `E`, the identifier must be escaped with RSmPL literal-token quoting. RSmPL uses backtick quoting for literal tokens whose spelling would otherwise be interpreted as a metavariable:

~~~diff
@@ -50,8 @@ rule=literal_error_type
# type T
# type E
- Result<T, `E`>
+ MyResult<T>
~~~

Here, `` `E` `` means the literal Rust identifier `E`, not the declared metavariable `E`.

Literal-token quoting is valid for identifiers, raw identifiers, lifetimes, labels, and path segments. It is not Rust syntax; it is RSmPL pattern syntax and must be removed during pattern parsing before the concrete Rust AST is matched.

Backticks quote one complete Rust lexical token. For a Rust raw identifier, the `r#` prefix is part of the quoted token. Therefore the canonical escaped spelling for the raw identifier `r#type` is:

~~~rust
`r#type`
~~~

The following form is invalid because the quoted token is incomplete and the raw-identifier prefix has been split from the identifier token:

~~~rust
r#`type`
~~~

Engine-verbatim quoting rejections (all exit 2, worktree untouched).
Parse-time triple — directive-parse stage, before any `rule:`/`file:`
is attached (text carries no `rule:`/`file:`/`range:`/`suggestion:`
lines; JSON carries explicit `rule: null` / `file: null` /
`range: null` / `suggestion: null`):

```text
RSMPL_PATTERN_CONFLICT
reason: invalid raw-identifier quoting `r#` + backtick in pattern ` Result<T, r#`type`>` (use `` `r#...` ``)
```

```text
RSMPL_PATTERN_CONFLICT
reason: invalid raw-identifier quoting in directive `# expr r#`E``
```

```text
RSMPL_PATTERN_CONFLICT
reason: unbalanced backtick quoting in pattern ` Result<T, `E>`
```

`{text}` is the full body-line text after the diff prefix is stripped
(leading space kept — `- Result<…>` reports `` ` Result<…>` ``);
`{line}` is the raw `#` directive line. The raw-split pattern reason
always carries the `(use `` `r#...` ``)` tail; the directive variant
has no tail.

Later-stage quad — pattern-parse stage, carries `rule:`/`file:`
(text omits null `range:`/`suggestion:` lines, JSON explicit nulls):

```text
RSMPL_PATTERN_CONFLICT
rule: x
file: src/lib.rs
reason: empty backtick quote ```` in pattern
```

```text
RSMPL_PATTERN_CONFLICT
rule: x
file: src/lib.rs
reason: backtick quote must hold a single token, got `foo bar`
```

```text
RSMPL_PATTERN_CONFLICT
rule: x
file: src/lib.rs
reason: invalid raw identifier in backtick quote `r#123`
```

```text
RSMPL_PATTERN_CONFLICT
rule: x
file: src/lib.rs
reason: invalid literal token in backtick quote `123`
```

`{inner}` is only the offending `` `…` `` token, never the full
pattern. All seven carry a null suggestion (omitted from text,
explicit `suggestion: null` in JSON).

Unquoted `r#type` remains valid Rust pattern text and matches a literal raw identifier named `type`; it is not interpreted as a metavariable because `r#type` is not a valid metavariable name. Use `` `r#type` `` when the author wants the literal-token intent to be explicit or when the raw identifier appears in a position where the implementation might otherwise apply metavariable-name normalization.

Raw-identifier matching is token-preserving by default. A pattern token written as `r#type` or `` `r#type` `` matches a source token spelled `r#type`, not an ordinary keyword token `type`. Semantic symbol matching may still resolve both through Rust name resolution when a `# symbol` directive is used; literal-token matching does not perform that normalization.

Examples:

~~~rust
`E`
`T`
`foo`
`r#type`
`'a`
crate::`E`::new
crate::`r#type`::new
~~~

If a backticked token names a declared metavariable, the backticked form always means the literal token. The unquoted form means the metavariable.

Therefore:

| Pattern token | Meaning when `# type E` is declared |
|---|---|
| `E` | type metavariable |
| `` `E` `` | literal Rust identifier `E` |

If a token is not declared as a metavariable, both `Foo` and `` `Foo` `` match the literal identifier `Foo`; the quoted form is allowed but unnecessary. For raw identifiers, both `r#type` and `` `r#type` `` match the literal raw identifier token `r#type`; the quoted form is allowed and is the canonical explicit escape.

### 15.5 Sigiled metavariable mode

v0.1 implements both modes. The default is:

```diff
# metavars=bare
```

Bare mode is more token-efficient and is the default for LLM-generated patches. Sigiled mode is available for tools or authors that prefer zero shadowing ambiguity. In bare mode a declared name is the metavariable reference (`E`); a `$`-prefixed spelling (`$E`) is not special — the `$` survives into the rewritten file and the reparse fails (mismatch fence below).

The stricter optional mode is:

```diff
# metavars=sigiled
```

In sigiled mode, metavariables must be written with a leading `$` in the pattern body:

```diff
@@ -50,8 @@ rule=sigiled_example
# metavars=sigiled
# expr E
- old($E)
+ new($E)
```

In this mode, unprefixed `E` always means a literal Rust identifier. A sigiled hunk that writes bare `E` therefore matches only a literal `E` in the target; against any other target it fails with the standard `RSMPL_MATCH_CONFLICT` no-match diagnostic (rule + file + old_range; Section 25.4):

```text
RSMPL_MATCH_CONFLICT
rule: r
file: a.rs
old_range: 1..2
reason: no match found — nearest source at line 1: 'fn f() { let x = 1 + 2; }'
suggestion: read `a.rs` to list valid ranges; check the pattern or add `# optional`
```

The quoted source line echoes the nearest candidate and varies per target (parameterized like Section 25.1); the JSON envelope carries the same `reason`/`suggestion` strings with `rule`/`file`/`range` set. Conversely, a `$`-prefixed spelling in bare (or default) mode is not a metavariable reference: the rewrite carries the literal `$` into the output and the rewritten-file reparse fails with `RSMPL_PARSE_CONFLICT` (file-only envelope — text omits the null `rule:`/`range:`/`suggestion:` lines, JSON carries explicit nulls; the `: {parser error}` suffix varies; canonical fence in Section 25.2):

```text
RSMPL_PARSE_CONFLICT
file: a.rs
reason: rewritten file does not parse as Rust: expected an expression
```

Explicit `# metavars=bare` behaves identically to the default. Success controls: sigiled `$E` against `1 + 2` applies cleanly, and sigiled bare `E` against a literal `E + 2` applies cleanly (proving the literal-identifier sentence).

### 15.6 Use examples

Expression metavariable:

```diff
@@ -50,8 @@ rule=wrap_expr
# expr E
- old(E)
+ new(E)
```

Type metavariable:

```diff
@@ -50,8 @@ rule=box_type
# type T
- Vec<T>
+ Box<[T]>
```

Item metavariable:

```diff
@@ -20,12 @@ rule=cut_item
# item I
# bind=CutItem
- I
```

Pattern metavariable:

```diff
@@ -50,8 @@ rule=if_let_to_matches
# pat P
# expr E
- if let P = E {
+ if matches!(E, P) {
      ...
  }
```


### 15.7 Repeated metavariable occurrences

If the same metavariable appears more than once in a single match pattern, the first occurrence binds the metavariable and every later occurrence must match the same bound value by structural equality.

This rule is mandatory. Repeated metavariables are not independent wildcards.

Example:

```diff
@@ -40,1 @@ rule=dedupe_assert_eq
# expr E
- assert_eq!(E, E)
+ assert_equal_once(E)
```

This matches:

```rust
assert_eq!(foo(), foo())
```

It does not match:

```rust
assert_eq!(foo(), bar())
```

For AST metavariables, structural equality means equality of the matched AST after insignificant trivia normalization. Whitespace, comments outside the captured node, and trailing comma normalization do not make two otherwise identical captures different. Distinct expressions such as `foo()` and `bar()` are not structurally equal.

Cross-statement correspondence (WI-5c): in a statement-sequence pattern (Section 16.4.2) the binding set is shared across all K statements, so a hole repeated in two statements must bind structurally-equal text in both — and holes are independent per position by construction (`P0` in statement 1 never constrains `P1` in statement 2). The inferrer numbers per-position holes positionally for exactly this reason.

For `symbol` metavariables, repeated occurrences must resolve to the same semantic symbol. For `token_tree` metavariables in macro token-tree mode, repeated occurrences must contain the same balanced token-tree sequence after token-level trivia normalization.

In a `+` pattern, each occurrence of a bound metavariable inserts the same captured syntax. A `+` pattern may duplicate a captured value intentionally:

```diff
@@ -20,1 @@ rule=duplicate_arg
# expr E
- once(E)
+ twice(E, E)
```

If any repeated occurrence is inconsistent with the first binding, the candidate match is rejected rather than partially bound.

### 15.8 Inferred metavariables (diet)

An undeclared identifier matching `[A-Z][A-Za-z0-9_]*` (a single
uppercase letter or a leading-uppercase word) in a metavariable
position infers its kind from that position — no `# expr E` line is
required. Explicit declarations remain legal and must agree with the
inferred kind; disagreement and ambiguous positions fail fast with
`RSMPL_PATTERN_CONFLICT`.

Kind by position:

| Position | Inferred kind |
|---|---|
| `- I` alone (single-token minus) | `item` (`# root=` forces `type`/`expr`/`stmt` for that token) |
| fn-header name `N` (bare, free or method position) | `ident` (the WI-5 name hole: binds the site ident, replays it) |
| bare-pair item name `N` (bare `struct N` / `enum N` / …) | `ident` (the WI-10b evolution hole: binds per site, replays byte-identical; attr delta required) |
| fn-header declared `# ident X` in name position | `ident` |
| fn-header `G` (between name and `(`) | `generic_params` |
| fn-header `W` (after params/return) | `where_clause` |
| fn-header `-> T` (single preserved return) | `type` (only when `T` occurs on both sides; one-sided `-> String` stays a concrete literal) |
| two pre-`fn` tokens (`V Q fn`) | first `visibility`, second `qualifier_list` |
| one pre-`fn` token (`V fn`) | ambiguous (`visibility` vs `qualifier_list`) — declare explicitly |
| `let P: T = E;` | pattern `pat`, `: T` region `type`, `= E` region `expr` |
| call/method callee, receiver, method, bare args | `expr` (macro `!` arg names infer only when occurring on both sides — single-side uppercase idents stay literal deltas; macro names never infer; `use`-paths never infer) |
| type argument interiors (`Vec<T>`, `Box<[T]>`, `Fn(T)` inputs and `-> T` outputs) | `type` for the inner names only; the leading constructor (`Vec`, `Box`, `Fn`) stays literal, as do one-sided concrete additions (`+ Send`) |
| otherwise (`# root=type/expr`) | the forced kind; unforced general text follows `try_general` routing (type-parseable → `type`, else `expr`) |

Rules:

- A bare `N` outside the fn-header and bare-pair item name positions
  never infers a new
  kind: in call/method and `let`/general expression positions it stays
  the literal ident `N`, while in type-argument and return positions the
  pre-existing diet type inference still applies (`foo::<N>(E)` binds
  `N` as `# type N`, T145). Explicit declarations always bind
  (`# expr N` + `foo(N)` is an expression hole); inference just refuses
  to invent a second kind beside `ident`.
- A name hole never binds in call/method callee positions (Section
  18.6 C20) and never binds qualified paths.
- Backtick quoting is unchanged: `` `E` `` is always the literal token
  and never infers. String literals and attribute/doc-comment lines
  never infer either.
- Sigiled mode (`# metavars=sigiled`) is unchanged: only `$Name`
  counts; bare `Name` is literal.
- `# symbol` names never infer (they are bound by declaration).
- One name in two kinds is `RSMPL_PATTERN_CONFLICT` naming both
  (`metavariable `E` occurs in incompatible positions (expr vs type)`);
  an explicit declaration naming a different kind than the inferred
  one is the same conflict (`metavariable `T` declared as expr but
  used as type`). An explicit declaration disambiguates the single
  pre-`fn` case (`# visibility V` + `V fn` is `visibility`).
  `ELLIPSIS` / `ELLIPSIS_*` never infers and never declares (WI-5b-follow):
  the placeholder would read as a `...` marker, so both paths fail fast
  with `reserved name`.

Engine-verbatim (both exit 2; `rule:`/`file:` lines; text omits the
null `range:`/`suggestion:` lines, JSON carries explicit nulls):

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: src/lib.rs
reason: metavariable `E` occurs in incompatible positions (expr vs type)
```

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: src/lib.rs
reason: metavariable `T` declared as expr but used as type
```

Diet patches need no per-metavariable declaration lines; hunk
detection is `rule=` or any `#` line (SPEC 7.1, WI-1d) — a
directive-free rename uses the terse form plus `rule=`.


---

## 16. Ellipsis Semantics

`...` is an explicit wildcard for syntax in defined positions.

It never implicitly matches missing visibility, missing qualifiers, missing generics, or a missing where clause. Attributes and doc comments are governed separately by the `# attrs=...` policy described in Section 14.6.

### 16.1 Supported ellipsis positions

| Pattern form | Meaning |
|---|---|
| `fn foo(...)` | any parameter list |
| `{ ... }` | any block contents |
| `foo(...)` | any argument list |
| `receiver.method(...)` | any method argument list |
| `S { ... }` | any struct fields |
| `match x { ... }` | any match arms |
| `impl T { ... }` | any impl items |
| `trait T { ... }` | any trait items |
| `crate::...::foo` | any path segment sequence |
| `[ ... ]` | any array elements |
| `( ... )` | any tuple elements depending on context |
| `{ prefix; ... }` | remaining block statements after a concrete prefix (general patterns only, Section 16.4.1) |

### 16.2 Function ellipsis

In:

```rust
fn foo(...) {
    ...
}
```

the first ellipsis means any parameter list.

The body ellipsis means any body.

The first ellipsis does not match:

- `pub`
- `async`
- `unsafe`
- `const`
- `extern`
- generic parameters
- where clauses
- attributes, which are governed separately by `# attrs=...`

### 16.3 Argument ellipsis

Pattern:

```rust
foo(...)
```

matches:

```rust
foo()
```

```rust
foo(a)
```

```rust
foo(a, b, c)
```

It does not match:

```rust
bar(a)
```

Each `...` in the `+` side replays, in order, the target args the
matching `- ...` absorbed: `- foo(E, ...)` / `+ bar(E, ...)` rewrites
`foo(1, 2, 3)` to `bar(1, 2, 3)`. Splits are tried left-to-right
(greedy preference) with backtracking: a failed tail retries later
splits, so `- foo(..., E)` / `+ bar(E)` rewrites `foo(1, 2, 3)` to
`bar(3)`, and `- foo(A, ..., B)` / `+ bar(A, B)` rewrites it to
`bar(1, 3)`. A `...` that absorbs nothing replays
empty and swallows one adjacent comma, so the same hunk rewrites
`foo(1)` to `bar(1)`. Omitting `...` on the `+` side is a deliberate
arg-drop (`- foo(E, ...)` / `+ bar(E)` rewrites `foo(1, 2)` to
`bar(1)`). A `...` in `+` with no matching `- ...` is a pattern error:
there is nothing to replay. An arg merely *containing* `...` inside a
larger expression (e.g. an array literal `&["--patch", M, ...]`) is not an
arg-level `...`: the call router treats that arg as concrete, the
placeholder bypass routes the hunk to the general matcher, and the general
matcher rejects `...` as a pattern error — never a silent literal emission
(T442).
`...` inside a plus-side string literal, char literal, or comment is concrete
text (WI-5b-follow, same discipline as trailing-block tails): `- foo(E, ...)`
/ `+ bar("...", E, ...)` rewrites `foo(x, 1)` to `bar("...", x, 1)` — only
the code-level `...` replays the absorbed remainder.
  The engine reports this verbatim (rule + file lines; text carries the suggestion line, JSON explicit nulls):
  ```text
  RSMPL_PATTERN_CONFLICT
  rule: r
  file: src/lib.rs
  reason: `...` in `+` has no matching `- ...` to replay
  suggestion: drop the `+ ...` or add a `- ...` absorbing the replayed args
  ```

A `...` fused to identifier characters is a different pattern error from
the replay fence above: `...` must stand alone (or sit inside delimiters),
so `foo(E...E)` never parses as a callee plus wildcard. The engine reports
this verbatim (rule + file lines; text omits the null `range:`/`suggestion:`
lines, JSON carries explicit nulls):

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: src/lib.rs
reason: `...` must be a standalone wildcard, got ` foo(E...E)`
```

`{line}` echoes the full body-line text after the diff-prefix strip
(leading space kept — `- foo(...)` reports `` ` foo(...)` ``).

### 16.4 Body ellipsis

Pattern:

```rust
{
    ...
}
```

matches any block body, including an empty body.

#### 16.4.1 Trailing-statement ellipsis in general patterns (WI-5b)

A general-pattern block (`if`/`while`/`for`/`loop` bodies, `match` arm
bodies, block expressions, `let-else` diverge blocks, closure bodies)
may end with one final pseudo-statement `...`:

```rust
if E {
    foo();
    ...
}
```

The `...` absorbs any remaining target statements (zero or more) after
the concrete prefix matched pairwise. The plus side carries `...` in the
same block to replay the absorbed tail verbatim, or omits it to
deliberately drop the tail (mirroring the Section 16.3 argument
replay/drop discipline). Replay is all-or-nothing in v1: the plus side
carries either every minus `...` (same count, same block structure, tails
replay in minus block order) or none (all tails dropped); anything
between fails at compile time, as does a plus `...` with no matching
minus `...`. No other `...` position is admitted in general patterns
(mid-block, second trailing, arm-level, operand, type, pattern) — those
fail at C1. Tails slice original source (indent included); a zero-tail
match replays empty. `# repeat` composes per site (tails are per-match).
A `"..."` string literal is concrete text, never a tail marker.

#### 16.4.2 Statement sequences (WI-5c)

A general-pattern hunk may instead hold K = 2–4 plain `let` statements
on each side (no attributes, init present, no `let-else` diverge —
Section 18.6 C18 admits exactly this shape; any other multi-statement
minus fails at C18). The engine matches K consecutive `Block`
statements (function bodies and nested blocks; file top-level items are
never `Block` members, so sequences never span items) with one shared
binding set across the K positions (Section 15.7 cross-statement
correspondence); windows never overlap (Section 17.6 posture — a
partially overlapping candidate fails loud). Each window position
matches with the existing structural `let` matcher, so per-statement
guards (`P: T` shapes, embedded-metavariable vetoes, ascription
agreement) apply unchanged; the plus side instantiates each line under
the shared bindings, preserving every target line's indent (the window
span starts at the first statement, so its line indent rides verbatim
and continuation lines re-apply theirs). Sequences are exact-K in v1:
no `...` inside members (C1), no `# repeat` (Section 17.8), no
deletion of a subset (an empty `+` side deletes all K statements,
whole-window). The inferrer emits one `# allow=many` hunk per
repeated-block class (Section D.4 `sequence` detector): N ≥ 2 blocks
whose statement runs anti-unify position-wise, with per-position holes
numbered positionally (`P0`/`E0` in position 0, `P1`/`E1` in position
1, …) so windows bind independently.

### 16.5 Path ellipsis

Pattern:

```rust
crate::...::foo
```

may match:

```rust
crate::foo
```

```rust
crate::a::b::foo
```

The exact zero-or-more behavior for path ellipsis must be documented by the implementation. The recommended behavior is zero or more path segments.

> **v0.1 implementation note:** on call callees (e.g. `- crate::...::old(E)`)
> the minus-side `...` matches zero or more path segments, and the
> surrounding segments still match exactly. The plus side must be concrete
> (`+ crate::new(E)`): a `+` path `...` has no absorbed path segments to
> replay (only argument remainders replay, Section 16.3) and is rejected
> with `RSMPL_PATTERN_CONFLICT`.


Engine-verbatim (exit 2; `rule:` + `file:` + set `suggestion:`; text omits the null `range:` line, JSON carries explicit `range: null`):

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: src/lib.rs
reason: `...` in `+` path has nothing to replay (plus side must be concrete, SPEC 16.5)
suggestion: write a concrete `+` path (e.g. `+ crate::new(E)`) — path `...` replays nothing, only argument `...` replays
```

Unlike the Section 16.3 argument-`...` fence (which replays absorbed call args), a path `...` has nothing to replay — the plus path must name every segment concretely.

---

## 17. Matching Semantics

### 17.1 Matching pipeline

For each RSmPL hunk, the engine should:

1. Identify the target file from the file section.
2. Identify the old-side semantic search range from the hunk header.
3. Parse the target file as Rust.
4. Parse the RSmPL hunk body as a Rust pattern.
5. Restrict the candidate AST nodes by range and scope.
6. Match concrete syntax, metavariables, repeated metavariable equality constraints, and ellipses.
7. Apply semantic constraints such as symbol resolution.
8. Verify match count against `allow` and `optional`.
9. Rewrite matched AST.
10. Reparse or otherwise validate the result.

### 17.2 Range filtering

The engine must not search outside the declared old-side range except as needed to parse a larger syntactic unit that overlaps the range.

For example, if the range points to the middle of a function and `scope=anchor` is used, the engine may parse the whole function but must only accept matches whose anchor is inside the range.

### 17.3 Scope modes

#### `scope=contained`

Default.

The matched AST node must be fully contained in the old-side range.
Probed shape: an `fn` item spanning lines 1-4 is not matched by a mid-body `@@ -2,2 @@` range (node not fully inside); a multiline call spanning lines 2-4 is not matched by a first-line `@@ -2,1 @@` range either.

#### `scope=anchor`

The first concrete non-ellipsis token in the pattern must be inside the old-side range.
Probed shape: the same mid-body `@@ -2,2 @@` range does not match the `fn` either (anchor sits on the `fn` keyword line, outside the range); a multiline call's first line `@@ -2,1 @@` matches (anchor inside), while its middle line `@@ -3,1 @@` does not.

Useful for matching a call inside a larger expression or statement.

> **v0.1 implementation note:** the anchor is approximated as the `fn` keyword line for function items and the candidate start line for call/macro matches.

#### `scope=overlap`

The matched AST node may overlap the old-side range.
Probed shape: the mid-body `@@ -2,2 @@` fn range matches, and so does a middle-line `@@ -3,1 @@` range over a multiline call — any overlap suffices.

Useful when the provided line range cuts through a larger node.

#### `scope=exact`

The matched AST node span must exactly equal the old-side range after trivia normalization.
Probed shape: a multiline call spanning lines 2-4 matches `@@ -2,3 @@` (full span) but not `@@ -2,1 @@` or `@@ -3,1 @@`; a single-line call matches its line range; an `fn` spanning lines 1-4 matches `@@ -1,4 @@` but not `@@ -2,2 @@`.

Useful for precise item movement.

#### `scope=file`

Search the whole file. The `@@ -old,len` old range is a dummy
(canonical `@@ -1,1 @@`) and range-bounds validation is skipped for
that hunk, so no range discovery is needed when the agent
already holds the file. Matching, `allow`/`optional` count
validation, and edit-level overlap checks are unchanged; only the
range pre-filter is lifted. Insertion anchors (`-old_start,0`) still
use the header as the anchor position — scope is moot there.

### 17.4 Repeated metavariable occurrences

A repeated metavariable occurrence is an equality constraint on the current candidate match.

The engine must bind the metavariable on its first occurrence and then require every later occurrence of the same metavariable to be structurally equal to the first bound value, as defined in Section 15.7.

This pattern:

```diff
@@ -40,1 @@ rule=same_assert_args
# expr E
- assert_eq!(E, E)
+ assert_equal_once(E)
```

matches `assert_eq!(foo(), foo())` and does not match `assert_eq!(foo(), bar())`.

If a candidate fails a repeated-metavariable equality check, the engine must discard that candidate and continue searching. If no valid candidates remain, ordinary match-count validation applies.

### 17.5 Match counts

Default:

```text
# allow=one
```

Exactly one match is expected.

If zero matches are found, the hunk fails unless `# optional` is set.

If more than one match is found, the hunk fails unless `# allow=many` is set.

The two flags are independent: `# optional` suppresses only the
zero-match failure, never the too-many failure. An `# optional` hunk
that finds two matches still fails with the verbatim too-many fence
(`RSMPL_MATCH_CONFLICT`, `reason: expected one match, found 2`,
``suggestion: add `# allow=many` if multiple rewrites are intended``;
rule + file + `old_range`, exit 2, text + JSON — Section 25.4). An
`# optional` hunk that finds zero matches succeeds with an empty
lowering: the JSON report carries `matches: 0` with `spans: []` and an
empty `diff` (text mode prints `(no changes)`; `check: true`). Combined
`# optional` + `# allow=many` therefore means zero-or-more (each flag
covers its own side).

### 17.6 Overlapping matches and `allow=many`

When `# allow=many` is set, matching is performed as a single top-down, source-order, non-overlapping pass.

The engine must:

1. walk candidate AST nodes in preorder, from outer nodes to inner nodes;
2. consider candidates in source order for nodes at the same depth;
3. accept the first candidate that matches and does not overlap an already accepted match;
4. skip all descendants of an accepted match for that hunk;
5. reject the hunk if two accepted candidates would partially overlap without one fully containing the other.
 Engine-verbatim (exit 2; `RSMPL_MATCH_CONFLICT`, `rule:` + `file:` lines; text omits the null `range:`/`suggestion:` lines, JSON carries explicit `range: null` / `suggestion: null`; suggestion is null, but the failure carries an `overlap`-template `fix_pattern` repair — narrow the range or split the hunks):
 ```text
 RSMPL_MATCH_CONFLICT
 rule: r
 file: src/lib.rs
 reason: partially overlapping matches cannot be merged
 ```
 Any two AST nodes from one parse are nested or disjoint, so real Rust inputs hit outermost-wins skip (step 4) or disjoint-merge -- this reason is the defensive backstop for a non-nested overlap, and shares one spelling across the call, general-expression, general-type, and macro matchers. Contrast the plan-stage `overlapping edits cannot be merged` (`GIT_HUNK_CONFLICT`, file-only plus a set suggestion, Sections 10.6/23.5/25.7), which fires when edits from different hunks overlap.

This means **outermost matches win**. The engine must not repeatedly re-run the same hunk until a fixpoint is reached. A hunk is one pass unless the specification later defines an explicit repeat directive.

Example:

```diff
@@ -10,1 @@ rule=unbox_many
# allow=many
# expr E
- Box::new(E)
+ E
```

Given this source expression:

```rust
Box::new(Box::new(x))
```

the selected match is the outer call, with `E = Box::new(x)`. The result is:

```rust
Box::new(x)
```

The result is not `x`, because the inner `Box::new(x)` is a descendant of the accepted outer match and is skipped during the same hunk application.

To rewrite both layers, the patch author must emit two ordered RSmPL hunks or add `# repeat` (Section 17.8).

For safety, implementations should report the number of accepted matches for every `allow=many` hunk.


### 17.7 Trailing commas and separator trivia

Trailing commas are insignificant for matching in Rust syntactic lists where Rust permits them.

The engine must treat trailing commas as separator trivia during matching for at least the following list-like forms:

- function and method argument lists;
- function, method, closure, and trait-item parameter lists;
- generic argument and generic parameter lists;
- tuple, array, and tuple-struct elements;
- struct literal fields and struct item fields;
- enum variant fields;
- match arms;
- macro invocation delimiters when the macro body is parsed as Rust syntax rather than raw token trees.

Therefore, this pattern:

```diff
@@ -10,1 @@ rule=call_two_args
# expr A
# expr B
- foo(A, B)
+ bar(A, B)
```

matches both:

```rust
foo(1, 2)
```

and:

```rust
foo(1, 2,)
```

The formatter or pretty-printer selected by `# format=...` is responsible for canonicalizing trailing commas in rewritten output. A rewrite must not preserve or remove a trailing comma in a way that produces invalid Rust.

In `# macro=token_tree` mode, raw token-tree matching does not automatically treat commas as Rust list separators unless the implementation parses a nested token sequence as a Rust syntactic list. Token-tree wildcard behavior is defined separately in Section 21.4.

### 17.8 Nested re-application (`# repeat`)

Without `# repeat`, one hunk is one outermost-wins pass (Section 17.6):
nested matches need a second ordered hunk. With `# repeat`, the engine
folds nested matches into the same hunk by re-applying the same
minus/plus rule inside each captured binding, bottom-up, before
substitution:

```diff
@@ -10,1 @@ rule=unbox_nested
# repeat
# expr E
- Box::new(E)
+ E
```

Given `Box::new(Box::new(x))`, the outer match binds `E =
Box::new(x)`; the rule re-applies inside the binding (`E = x`); the
outer substitution then yields `x`. One hunk replaces two.

Rules:

- Only named-metavar bindings are revisited (pre-bound `# symbol`
  paths are skipped). A binding that does not parse as an expression is
  kept verbatim. `...` ellipsis bodies are opaque arg-list text, not a
  single expression: to let repeat reach inside, bind the positions
  with named `# expr` metavariables instead of `...`.
- The produced output is never re-scanned, so self-reproducing rules
  (`foo(E)` -> `foo(foo(E))`) apply exactly one level and terminate;
  recursion depth is bounded by the AST (backstop: 64 levels).
- The per-rule match count is the outermost accepted sites (one edit
  each); nested folds ride the outer edit and are visible in the diff.
  `allow`/`optional` validation is unchanged.
- In v0.1, only call/method-call and general-expression shapes accept
  `# repeat`; any other shape (function, item, `let`-statement, type,
  macro, insertion, deletion, capture) is rejected with
  `RSMPL_PATTERN_CONFLICT` (Sections 9.6, 24.4). `let`/type shapes mix
  patterns, types, and initializers in one binding set, so folding an
  expression rule inside a bound pattern or type would be surprising;
  macro shapes stay verbatim-only by design (Section 21). Extending
  repeat to other shapes is future work.

 The engine reports shape-specific reasons (rule + file lines; text
 omits the null range/suggestion lines, JSON carries explicit nulls
 unless noted):

 ```text
 RSMPL_PATTERN_CONFLICT
 rule: r
 file: src/lib.rs
 reason: `# repeat` is not supported on `let`-statement patterns in v1 (folding an expression rule inside a bound pattern or type would be surprising); emit one ordered hunk per nesting level
 ```

 Type patterns report `` `# repeat` is not supported on type patterns
 in v1; emit one ordered hunk per nesting level ``; statement-sequence
 patterns report `` `# repeat` is not supported on
 statement-sequence patterns in v1; emit one ordered hunk per nesting
 level ``; every other shape reports `` `# repeat` requires a
 call/method-call or general-expression rewrite pattern (other shapes
 are future work) `` (all suggestion null). A `# repeat` hunk that also declares
 `# bind=`/`# use=` reports `` `# repeat` cannot be combined with
 `# bind=`/`# use=` (captures move verbatim) `` with suggestion
 `remove `# repeat` from the cut/paste hunk`. A hunk with only one side
 (`-` without `+`, or `+` without `-`) reports `` `# repeat` requires
 both `-` and `+` patterns (insertions/deletions already cover nested
 matches) `` with suggestion `remove `# repeat`, or write a rewrite
 hunk`. Gate order: a valued spelling (`# repeat=<value>`) never reaches
 these gates — directive parsing rejects it first (verbatim reason +
 suggestion, Section 9.6) — and a combined bind/use hunk reports the
 bind/use fence above. Here `v1` names the general-matcher generation
 (the SPEC 18.6 C-table namespace), not the SPEC v0.1 version.

Past the shape gates, pathological nesting fails at match time with
`RSMPL_MATCH_CONFLICT` — both the call matcher and the
general-expression matcher emit the identical reason + suggestion pair.
Engine-verbatim (exit 2; `rule:` + `file:` lines; text omits the null
`range:` line, JSON carries explicit `range: null`; `suggestion:` is
set — contrast the suggestion-null shape gates above):

```text
RSMPL_MATCH_CONFLICT
rule: r
file: src/lib.rs
reason: repeat nesting exceeds 64 levels (pathological nesting?)
suggestion: remove `# repeat` or simplify the nesting
```

Probed 2026-10-27: a 65-deep `Box::new` nest still folds clean while a
70-deep nest fails, so the backstop trips strictly above the documented
bound — e2e pins use 70-deep (fail) plus a shallow fold control and
never the exact trip line.

---

## 18. Rewrite Semantics

### 18.1 Replacement

A hunk with matching `-` and `+` patterns replaces the matched AST fragment.

> **v0.1 implementation note:** function-item hunks support signature evolution, not just renames. Each explicitly written slot evolves independently: an explicit `-`/`+` slot difference inserts, replaces, or deletes that slot's source bytes while all other slots (including the body and, unless `...` is replaced by concrete text, the parameter list) are preserved. Slot comparison is whitespace-insensitive against target text sliced from the original source; a concrete `-` slot must equal the target or the candidate is rejected (fail over guessing). `...` (parameters) and the same metavariable on both sides (visibility, qualifiers, generics, where) mean "preserve". A concrete body on either side is a compile error (bodies are always preserved). Minus-concrete/plus-omitted deletes the slot for parameters-as-`...`-removed, generics, and `where` clauses; for the return type, minus-concrete/plus-omitted deletes `-> T`, while plus-omitted with minus-`...` preserves the return type (rename fast-path). There is no syntax to delete a return type while matching any return with `...`; `# returns=none` constrains only the `-` side. Impl-block methods (`impl` context hunks) share the same slot semantics, including `self`/receiver parameters.
>
> Bare `<kind> <Name>` pairs for `struct`, `enum`, `trait`, `union`, `type`, `const`, `static`, `mod`, and `impl` (Section 14.8) are item renames, not general shapes: one ident-span edit per match, everything else preserved. `impl` renames the self-ty ident and the trait-side path's last segment.
>
> Non-call shapes are handled by the generalized matcher. Statements are limited to `let` bindings (`syn::Stmt::Local`, with or without a `let-else` diverge block — both sides must agree structurally): the pattern may bind the whole pattern with one `# pat P` placeholder, use a concrete pattern, spell an ascribed shape `P: T` with placeholder parts, or spell `P @ sub` with a placeholder head (WI-5a: `P` binds the whole target pattern text, `sub` matches recursively). A bare `P` never matches an annotated `x: T` (strict, like function slots); embedded metavariables elsewhere in the pattern are rejected at compile time. The initializer is matched with the general expression matcher.
>
> General expressions cover paths, literals, binary/unary/cast/reference/paren, calls, method calls, assignment, field access, indexing, arrays, tuples, struct literals, ranges, `await`, `?`, `return`, `break` (optional label plus value), `continue` (optional label), `if` (condition plus then-block plus else branch — `else if` chains recurse), blocks (concrete prefix plus an optional trailing `...` tail, Section 16.4.1), `match` (scrutinee plus arms — arm patterns bind whole placeholders or compare strictly, guards and bodies match structurally, arm count must agree), `loop`/`while`/`for` (labels strictly equal, conditions/iterators matched structurally, bound patterns via `# pat P`, bodies as concrete prefixes with optional trailing `...` tails), closures (async/move/qualifier strict, params decomposed like `P: T`, return type and body structural), `async`/`try`/`unsafe` blocks (qualifiers strict, bodies structural) and `yield` (optional value structural, WI-6 slice 1), and `let` expressions (pattern plus scrutinee — the `if let`/`while let` condition form). Block members that are macro statements match only whole-shape (path, delimiters, and body token-equal — interiors are opaque and never host holes, Section 21); members that are `fn` items match with attributes, visibility, and signature strict and the body structural (holes may live in the inner function); any other item member must be token-equal (WI-6b). General types cover paths with generic arguments (including parenthesized `Fn(A) -> B` arguments, whose inputs match via `match_type` and whose output matches via `match_return_type` — cross-kind pairs such as `P(T)` vs `Vec<u8>` still reject), references, slices, arrays, tuples, parentheses, raw pointers (`*const`/`*mut`), fn-pointer types, `impl Trait`, `dyn Trait`, `_` (which matches any type), and macro types verbatim-only (token-equal, never holed — a `# type T` hole inside a macro never binds). Anything else (mixed `let-else` where only one side carries the diverge, `...` outside trailing-block position) rejects the candidate rather than guessing. (WI-6 slice 1: `async`/`try`/`unsafe`/`yield` join the control-flow family; WI-6b: block members as above; WI-6c: macro types verbatim + parenthesized `Fn` args.) Placeholder path segments may carry generic arguments (`P<u8>`, `P::<T>`) and method-call turbofish compares structurally (`collect::<T>` binds `T`); lifetimes are wildcards in v1 and are not compared. A call shape must consume the whole pattern: `foo(E) + 1` and `E.foo() + 1` route to the general matcher, not to a `foo(E)`/`E.foo()` call prefix (only a trailing `;` keeps call routing, so `foo(E);` stays a call). An empty `+` side deletes a `let` statement (whole lines) or a `...;` expression statement; type patterns cannot be deleted. Leading `#[...]` attributes are only supported on `let` patterns and follow `# attrs=...` (a `+` side that writes attributes the `-` side did not is rejected as ambiguous). The fail-fast reference is Section 18.6.

Example:

```diff
@@ -40,8 @@ rule=rename_call
# expr E
- old_api(E)
+ new_api(E)
```

### 18.2 Deletion

A hunk with only `-` semantic pattern lines removes the matched AST fragment.

Example:

```diff
@@ -20,12 @@ rule=delete_helper
# item I
- I
```

### 18.3 Insertion

A hunk with only `+` semantic pattern lines inserts the new AST fragment at the new-side range.

Example:

```diff
@@ -5,0 @@ rule=insert_use
# lang=rust
+ use crate::new_module;
```

### 18.4 Context-preserving rewrite

Context lines constrain the surrounding AST and are preserved.

Example:

```diff
@@ -20,12 @@ rule=rename_method_inside_impl
  impl Service {
      ...
-     fn old_name(...) {
+     fn new_name(...) {
          ...
      }
  }
```

### 18.5 Metavariable substitution

Metavariables bound in the `-` or context pattern may be reused in the `+` pattern.

Example:

```diff
@@ -40,8 @@ rule=method_to_function
# expr Receiver
# expr Arg
- Receiver.old_method(Arg)
+ new_function(Receiver, Arg)
```

If a metavariable is used in a `+` pattern without being bound or imported via `# use`, the hunk fails.
The receiver slot is all-or-nothing: a bare declared metavariable binds the whole receiver text, including chains (`- E.to_string()` binds `"a".trim()`), while a composite receiver pattern is compared verbatim (`- R.trim().to_string()` never matches a different site) — match the inner call first, or bind the whole receiver with one metavariable. A leading `&` is not part of the receiver at all: a `&`-led pattern (`- &E.join("")`, `- &dir.path()`) bypasses call routing and matches structurally as a general reference expression (Section 18.1 v0.1 note) — the reference wrapper never forces verbatim comparison.

### 18.6 v0.1 general-matcher coverage table (fail-fast reference)

General `let`-statement, expression, and type patterns (Section 18.1, v0.1
note) cover a bounded shape set. Anything outside it fails fast: either at
compile time with the exact diagnostic below, or at match time as a silent
candidate rejection that surfaces as `RSMPL_MATCH_CONFLICT` with reason `no
match found` and a suggestion naming the file to read for valid ranges.
There is no partial matching and no guessing; check this table before
authoring instead of iterating against `--check`. The match-time (R-row)
rejections are permanent v0.1 scope, not deferred work: mixed `let-else`
(R2) and macro types (R1/R5) go
through ordinary hunks (the same composition as Section 21.3 macro
bodies and Section 22 formatting). Blocks, `if`, `match`,
loops, closures, `break`/`continue`, `let` expressions, agreed `let-else`,
`async`/`try`/`unsafe` blocks, `yield`, block macro statements and `fn`
items (WI-6 slice 1/WI-6b),
and `P @` subpatterns are implemented (WI-5a/WI-5b) with the exact semantics
in their rows. Reopen a remaining row only with a dogfood
case where an ordinary hunk cannot express the edit.

#### Compile-time rejections (exact diagnostics)

| # | Trigger | Exact diagnostic |
|---|---|---|
| C1 | `...` outside trailing-block position in a general minus/plus side (WI-5b: one final `...` per block is admitted, Section 16.4.1; WI-5c sequences are exact-K — no `...` inside members, Section 16.4.2) | `v1 general patterns support `...` only as trailing block statements (`{ prefix; ... }` replays the absorbed tail, omitting it drops the tail); any other `...` position needs a call pattern or a concrete shape` |
| C2 | (closed by WI-5a) `let-else` in the minus pattern now matches a `let-else` target structurally: both diverge blocks must be present and compare as `Block`s (concrete prefix plus optional trailing `...` tail, Section 16.4.1); mixed `let-else` (only one side carries the diverge) rejects at match time under R2 | — |
| C3 | `# root=stmt` with a non-`let` shape | `` `# root=stmt` requires a `let` statement pattern (v1 supports `let` with or without `let-else`) `` |
| C4 | `# root=expr` with a non-expression shape | `` `# root=expr` requires a Rust expression pattern `` |
| C5 | `# root=type` with a non-type shape | `` `# root=type` requires a Rust type pattern `` |
| C6 | empty `+` side on an expression | `v1 expression deletion (empty `+` side) is only supported for `...;` statement shapes; match a `let` statement or write an explicit replacement` |
| C7 | empty `+` side on a type | `v1 type patterns cannot be deleted (empty `+` side would produce invalid syntax); write an explicit replacement type` |
| C8 | embedded metavariable inside a bound pattern in any pattern position (`let`, match-arm, `for`, closure-param, `let`-expression): anything other than a whole-pattern `# pat P` placeholder, a `P @ sub` shape with a placeholder head and a single-placeholder-or-concrete subpattern, a concrete pattern, a WI-3 structural composite (`Some(P)`, `(P, Q)`, `S { f: P }`, `&P`, `[P]`, `(P)`) with holes only in bindable leaf positions, or `P: T` with placeholder parts (a `let` annotation whose every placeholder names a declared `# symbol` — `Vec<Old>`, `Option<Vec<Old>>`, `&Old` — passes: symbols verify structurally, never bind) | `v1 patterns support a whole-pattern `# pat P` placeholder, a concrete pattern, WI-3 structural composites (`Some(P)`, `(P, Q)`, `S { f: P }`, `&P`, `[P]`, `(P)`) with holes in bindable leaf positions, or `P: T` with placeholder parts, in `let`, match-arm, `for`, closure, and `let`-expression positions; embedded metavariables elsewhere in a pattern are not supported` |
| C9 | `# returns=none` on a non-function hunk | `` `# returns=none` requires a function-item pattern `` |
| C10 | leading `#[...]` attributes on a non-`let` shape | `` leading attributes are only supported on `let` statement patterns in v1 `` |
| C11 | `+` side writes attributes the `-` side did not match (outside the Preserve add delta: minus empty plus non-empty under `# attrs=preserve` adds, on fn/item/`let` shapes) | `replacement writes attributes the pattern did not match; write the same attributes in the `-` pattern or drop them` (suggestion: `` mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip` ``) |
| C12 | `+` attributes differ from matched `-` attributes (outside the Preserve remove/same-length-replace delta under `# attrs=preserve`, on fn/item/`let` shapes) | `replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges` (same suggestion as C11) |
| C13 | `# symbol` declared without `# fallback=syntax` in a call-shape hunk whose seeded path resolves to no definition in this crate | `` symbol `<Name>` (`<path>`) resolves to no definition in this crate `` (code `RSMPL_SYMBOL_CONFLICT`, suggestion `` write the concrete path `<path>` in the pattern, or add `# fallback=syntax` to allow syntactic matching ``, repair `add_directive` `# fallback=syntax`). Only minus-side seeds must resolve (plus-only seeds name the rewrite target, which the crate may not define yet); call shapes and general whole-path positions reach this gate (sequence/macro and non-whole-path uses fail earlier at C14). Envelope: text carries `rule:`/`file:` plus the reason and a set `suggestion:` line (null `range:` omitted, no `old_range:` line); JSON carries `rule`/`file` set, `range: null`, and the same set suggestion — the suggestion-set contrast vs the suggestion-null C14/C15 rows. |
  | C14 | `# symbol` outside call shapes, general symbol positions, field-type evolution, and use-prefix renames, or a symbol in a non-admitted slot (suffix segment `X::Old`, abutting args `Old<T>`, `qself` tail, method ident, `let`-pattern interior, field/label slot, statement sequence) | `` `# symbol` is supported in call shapes (call-callee, call-argument, method-receiver), general symbol positions (general-expression, general `let`-statement, general-type — whole-path and single-leading-prefix), field-type evolution (`struct S {` + `x: Old` — whole-path, nested, or single-leading-prefix type), and use-prefix renames (`use Old…` — whole-prefix or single-leading symbol head, SPEC 19.3) in v0.1 — symbols match by exact full-path text or crate-graph proof (SPEC 19.3); this SHAPE hunk cannot use `# symbol` — use a call-shape, general, field-type, or use-prefix symbol hunk, or a concrete path pattern `` with SHAPE one of `general statement-sequence`, `macro`, and every other non-call non-general/field-type/use shape (code `RSMPL_PATTERN_CONFLICT`, suggestion null — the `use a call-shape, general, field-type, or use-prefix symbol hunk, or a concrete path pattern` tail is part of `reason:` after ` — `, not a separate suggestion). A `# symbol` naming the *method ident* of a method or general method call is also rejected: the method slot compares bare idents, never seeded paths, so it could never match — `` `# symbol old` names the method ident; method idents already match exactly — remove that `# symbol` line (symbols verify whole paths, SPEC 19.3) `` (same code, suggestion null). Position rejections name their slot: `` `Old` names a path segment of a longer path — only a leading symbol segments a prefix (`Old::suffix`); suffix segments ride concrete paths in slice 2a (write the full path) ``, `` `Old` carries generic args without `::` — write `Vec<Old>` (nested) or `Old::<T>` (turbofish), or the full concrete path ``, `` `Old` names a non-leading path segment — only a leading symbol segments a prefix (`Old` must head the path); suffix segments ride concrete paths in slice 2a (write the full path) ``, `` `# symbol` in a `qself` tail (`<T as Tr>::…`) never resolves — qself sites are opaque; write the full concrete path ``, `` `# symbol` in a pattern interior (binding position) never verifies — patterns bind; write the concrete path in an expression or type position ``, `` `Old` names a field/label slot, not a path — field and label positions ride concrete paths in slice 2a ``, `` `Old` names a non-leading `use` segment — only a leading symbol heads a use prefix (`use Old::…`); suffix segments ride concrete paths (write the full path) `` (each code `RSMPL_PATTERN_CONFLICT`, repair `fix_pattern` code `C14`, suggestion null). Text output omits the null `range:`/`suggestion:` lines; the JSON envelope carries explicit `range: null` / `suggestion: null`. |
| C15 | inert directives: lone `# fallback=syntax` (no `# symbol`, no `# follow=uses`) or non-default `# attrs=` on a call/method-call/macro shape | `` `# fallback=syntax` without `# symbol` does nothing (call matching is already syntactic in v0.1) — remove the line, or add `# symbol Name=path` (SPEC 19.3)`` and `` `# attrs=` is ignored on call/macro shapes in v0.1 (attribute envelopes are only checked on fn/item/general/use shapes) — remove the line (SPEC 14.6)`` (code `RSMPL_PATTERN_CONFLICT`, suggestion null) |
| C17 | `# follow=uses` on any shape other than a fn, item, field, variant, assoc, trait-method, or inherent-method rename hunk (WI-6a, WI-9a, WI-27, WI-3, WI-6; evolution, member-addition, assoc-evolution, and `impl T for S`-header method kinds included) | `` `# follow=uses` is only supported on fn, item, field, variant, assoc, trait-method, and inherent-method rename hunks in v0.1 (other shapes keep the SPEC 14.8 follow-up hunks)` `` (code `RSMPL_PATTERN_CONFLICT`, suggestion set: `` remove `# follow=uses`, or rewrite this hunk as a fn/item/field/variant/assoc rename ``; same rule+file envelope as the C3/C4/C5 siblings — text omits the null `range:` line, JSON carries explicit `range: null`) |
| C16 | `# root=item` with a minus side that is not a single declared metavariable | `` `# root=item` requires a single-metavar item-capture pattern (write `- I` for a declared `# item I` metavariable) `` (code `RSMPL_PATTERN_CONFLICT`, suggestion null; same rule+file envelope as the C3/C4/C5 siblings — text omits the null `range:`/`suggestion:` lines, JSON carries explicit `range: null` / `suggestion: null`) |
| C18 | multi-statement minus outside the WI-5c sequence shape (WI-9c(c) narrowed): parses as two or more Rust statements that are not 2–4 plain `let` statements with an equal-count plain-`let` plus side (Section 16.4.2) | `` v1 statement sequences need 2–4 plain `let` statements on both sides with equal counts (the minus side parses as N Rust statements, not a plain-`let` sequence); write 2–4 plain `let` statements, one hunk per statement or item, or use an ordinary hunk `` — or, for a sequence-shaped minus with a mismatched plus side, `` v1 statement sequences need 2–4 plain `let` statements on both sides with equal counts (the minus side has K plain `let` statements, the plus side differs); write 2–4 plain `let` statements, one hunk per statement or item, or use an ordinary hunk `` (code `RSMPL_PATTERN_CONFLICT`, suggestion null; same rule+file envelope as the C3/C4/C5 siblings — text omits the null `range:`/`suggestion:` lines, JSON carries explicit `range: null` / `suggestion: null`) |
| C19 | hole inside a macro invocation body in a general minus pattern, or a `Verbatim` expression/type/pattern node in a pattern position (WI-2b) | `` v1 general patterns match macro invocations verbatim (a hole inside `NAME!(...)` never binds); write the macro body concretely, use a `# macro=token_tree` hunk for whole-macro rewrites or per-argument holes (WI-7a: one hole per top-level comma-separated argument), or use an ordinary hunk `` — or, for engine-opaque nodes, `` v1 general patterns do not match `Verbatim ...` nodes (engine-opaque); write a concrete shape or use an ordinary hunk `` (code `RSMPL_PATTERN_CONFLICT`, suggestion null; same rule+file envelope as the C3/C4/C5 siblings — text omits the null `range:`/`suggestion:` lines, JSON carries explicit `range: null` / `suggestion: null`) |
| C20 | a name hole (`N` / `# ident X`) in a call/method/macro callee position (WI-5) | `` `N` is a name hole (`# ident` / diet `N`); it only binds in fn-header name position — write the call/method callee literally (or `` `N` `` to match the literal ident, SPEC 15.8) `` with `` `N` `` the offending hole name (code `RSMPL_PATTERN_CONFLICT`, repair `fix_pattern` code `C20`, suggestion null; same rule+file envelope as the C3/C4/C5 siblings — text omits the null `range:`/`suggestion:` lines, JSON carries explicit `range: null` / `suggestion: null`) |
| C21 | a general Stmt/Expr template whose hole-erased concrete bound-ident sets differ across minus/plus `Pat` positions without binder identity — a plus-side binder minus never bound (multi-leaf destructuring renames, binder add/drop, concretions like `Some(P)` → `Some(x)`, attr-envelope + rename combos), or a dropped binder with no hole replay (`Some(x)` → `None`); pure permutations and hole replays (`P @ sub` → `P`, `Some(x)` → `Some(P)`) keep applying, plain bare-ident renames and Stmt single-leaf destructuring renames keep today's behavior through binder identity, and sequences are exempt except a rename-once `let` rename (plain or single-leaf destructuring) at any single position, which enters binder identity (hetero fusion owns other binder-renaming runs there; authored init-substitution of later in-window uses completes by authoring, spelled-`old` replays strand as orphans (per-occurrence exclusion; hole-bound shadow/macro olds surface), partial swaps plus `new`/pat mentions stay declined window-only, and a Stmt single-`let` statement or sequence window whose enclosing scope already binds `new` outside the statement/window declines follow with `capture-risk` orphans) | `` general template rewrites bound binder idents without binder identity; v1 general templates never own binder renames (C21) `` (code `RSMPL_PATTERN_CONFLICT`, repair `fix_pattern` code `C21` with `write_instead` `` rewrite as a single-binder hunk (plain `- let old = E;` / `+ let new = E;` and friends, SPEC 14.8) or use an ordinary hunk ``, suggestion null; same rule+file envelope as the C3/C4/C5 siblings — text omits the null `range:` line, JSON carries explicit `range: null`) |

C6 note: `...;` expression-statement deletions never reach the general
matcher — call-like `foo(E);` shapes route to the call matcher (Section 13.4)
— so C6 fires only for empty-`+` general-expression shapes. A minus-only
call-shape hunk deletes the matched call expression instead (Section 18.2):
a statement site keeps its `;` (`foo();` → `;`), while a nested deletion
that would produce invalid syntax fails at rewritten-file reparse
validation (Section 24.2). Delete a `let` statement instead (whole
lines), or write an explicit replacement.

Routing note: `let`-led patterns always reach the `let` gate — a
`let`-keyword callee guard stops the call router claiming `let (E, x)`
as a `let(...)` call (likewise `let [E, x]` and `let x = foo(E)`).
`let (E, x) = ...` therefore fails at gate C1 (`...` rejected) and
`let (Some(E) | None) = E;` at gate C8 (`Or`-with-hole — binding under
alternation is ambiguous), not with a bogus
`no match found` from the call matcher. WI-3 admits tuple/struct holes,
so `let (E, x) = E;` now compiles to a structural `Stmt` match. Callees containing whitespace or
a statement keyword (`if`/`match`/`return`/...) likewise fall through to
the general matcher, as does any `&`-led pattern (`&E.join("")`,
`&dir.path()`): the leading `&` is a reference wrapper, never a call
receiver, so these match structurally as general expressions (Section
18.5) with or without `# root=expr`. A call shape must additionally consume the whole
pattern (balanced parens, then only whitespace or a lone `;`): `foo(E) + 1`
and `E.foo() + 1` route to the general matcher rather than a `foo(E)` /
`E.foo()` call prefix, while `foo(E);` keeps call routing.
The receiver/method dot split skips `"..."` string literals (T419): a dot
inside a string arg (`R.contains("a.b")`) never splits receiver from method.

Envelope: all twelve live C-table rejections above fail with
`RSMPL_PATTERN_CONFLICT` (exit 2; `rule:` + `file:` lines; text omits the
null `range:`/`suggestion:` lines, JSON carries explicit `range: null` /
`suggestion: null`) -- except C11/C12, which carry the set suggestion
(like the Section 14.6 pair they share strings with). C2 is closed by
WI-5a (now a firing shape, pinned end to end) and no longer fails; C19 is the WI-2b macro-hole/`Verbatim` gate; C20 is the WI-5 name-hole callee gate.

#### Match-time silent rejections (surface as `no match found`)

| # | Trigger | Mechanism | Write instead |
|---|---|---|---|
| R1 | (closed by WI-5a/WI-5b/WI-6 slice 1; extended by WI-2a) macro invocations and everyday expressions as the general expression | bounded variant set: paths (leading `::` compares with parity and `qself` qualifiers compare token-equal — WI-7d, placeholders never bind inside `qself`), literals, binary/unary/cast/reference/paren, calls, method calls, assignment, field access, indexing, arrays, tuples, struct literals, ranges, `await`, `?`, `return`, `break`/`continue` (labels strict), `if` (condition, then-block as a concrete prefix plus optional trailing `...` tail, else branch — `else if` chains recurse), blocks (concrete prefix matched pairwise plus an optional trailing `...` tail absorbing the rest, Section 16.4.1; macro-statement members match only whole-shape token-equal with no holes, `fn`-item members match with a strict header plus a structural body, and any other item member must be token-equal — WI-6b), `match` (scrutinee plus equal-count arms in order; arm pattern structural like `let` — whole-`P`, `P @ sub`, or WI-3 composites — else strict token-equal, guard/body structural), `loop`/`while`/`for` (labels strict; `for` bound pattern structural like `let`), closures (qualifiers strict; params structural like `let` — `P: T` parts admitted; body structural), `async`/`try`/`unsafe` blocks (qualifiers strict; bodies structural — trailing `...` rides along), `yield` (optional value structural), `let` expressions (pattern plus scrutinee), verbatim macro invocations (token-equal via `render_eq`, never holed — a hole inside a macro never binds), array repeats (element plus length recurse), `&raw` pointers (mutability strictly equal, pointee recurses), `const` blocks (compared via `match_block`), `_` (matches any expression, binds nothing); anything else rejects the candidate | an ordinary Git hunk, or a call/expr shape around the inner expression |
| R2 | mixed `let-else` (only one side carries the diverge block) | agreed `let-else` matches structurally (WI-5a: both diverge blocks present, compared as `Block`s); mixed presence rejects the candidate | an ordinary Git hunk |
| R3 | (closed by T144/T145; extended by WI-6c) placeholder path segments with generic arguments match structurally (`P<u8>`, `P::<T>` bind the segment ident and compare args via `match_generic_arg`); method-call turbofish compares structurally too (lifetimes wildcard, `_` matches any type; T176: call-shape `R.collect::<T>()` binds through the call router, not only via the general matcher; placeholder `R.M::<T>()` binds the method ident like callee `P::<T>`, and bare `R.M()` never matches a turbofish site (T177 parity, T178)). T177: a bare call/method pattern without `::<...>` never matches a turbofish site (None-vs-Some fails on both arms, general-matcher parity) — spell `::<T>` explicitly. Function-call turbofish compares structurally as well (`id::<T>` binds `T` to the target source text; concrete args compare token-normalized; the pattern must use the `::<>` form — a bare `id<T>` pattern never matches a real call). WI-6c: parenthesized `Fn(A) -> B` arguments match structurally (inputs pairwise via `match_type` so `# type T` holes bind, output via `match_return_type`; arity and output-shape mismatches reject) | cross-kind pairs (`P(T)` vs angle-bracketed `Vec<u8>`, any None/Angle/Parenthesized mix) still reject via rendered-text comparison | — |
| R4 | `P @ sub` with a concrete head, or a composite-placeholder subpattern | WI-5a: a placeholder head `P` binds the whole target pattern text and `sub` matches recursively (a target `@` unwraps to its subpattern, otherwise the whole target pattern; single-placeholder subs bind, concrete subs compare strictly token-equal); WI-5b: the same rule applies in match-arm, `for`-loop, closure-param, and `let`-expression pattern positions; WI-3: the six structural composites (`Some(P)`, `(P, Q)`, `S { f: P }`, `&P`, `[P]`, `(P)`) match in all of those positions too, binding holes per position through the shared core (Section 15.1); write plus sides with `P` alone (repeating the `P @ sub` spelling re-splices the whole target text and duplicates the subpattern); non-placeholder heads and composite subs fail at C8 | a whole-pattern `# pat P` placeholder or a concrete pattern |
| R5 | (narrowed by WI-6c) macro types as the general expression — engine-opaque, now verbatim | supported: paths with generic arguments (including parenthesized `Fn(A) -> B` args, WI-6c; type-position `qself` compares with parity like R1 expression paths — `<T as Tr>::assoc` matches only identical-qualifier sites, never qualifier-free `Tr::assoc` and vice versa), references, slices, arrays, tuples, parentheses, raw pointers (`*const`/`*mut`, constness strict), fn-pointer types (arity/unsafety/ABI strict, variadics never match), `impl Trait`, `dyn Trait` (bound count strict, `dyn` presence strict, `?` modifiers and `for<'a>` binders strict, plain lifetime bounds wildcard), `_` (which matches any type), `!`, macro types verbatim-only (token-equal via `render_eq`, never holed — a `# type T` hole inside a macro never binds; mixed macro/non-macro rejects); anything else rejects the candidate | a concrete type or a whole-type `# type T` placeholder |

#### Deliberate wildcards (not failures)

- Lifetimes are wildcards in v1: generic-argument and reference lifetimes
  are never compared.
- `_` as a type matches any type.
- Trailing commas are insignificant wherever Rust permits them (Section 17.7).
- A bare `P` never matches an annotated `x: T` — spell the ascription
  explicitly as `P: T` (strict, like function slots). A repeated
  metavariable must bind structurally equal values at every occurrence
  (Section 15.7); disagreement rejects the candidate.

---

## 19. Semantic Symbol Matching

RSmPL supports semantic symbols for precise renames.

### 19.1 Symbol directive

```text
# symbol Old=crate::old_api
```

This declares `Old` as a semantic symbol metavariable.

### 19.2 Symbol rewrite example

```diff
@@ -80,20 @@ rule=rename_symbol_calls
# allow=many
# fallback=syntax
# symbol Old=crate::old_api
# symbol New=crate::new_api
- Old(...)
+ New(...)
```

The engine must:

1. find calls whose callee path text equals `crate::old_api` exactly (v0.1 syntactic fallback compares full paths; true resolution is future work);
2. rewrite them to `crate::new_api`;
3. avoid changing unrelated identifiers with the same spelling.

### 19.3 Required semantic environment

Symbol resolution is crate-local from `syn` alone (no build, no cargo):

- module graph: file paths (`src/foo.rs` → `crate::foo`,
  `src/foo/mod.rs` → `crate::foo`) plus inline `mod` nesting;
- item definitions per module (`fn`/`struct`/`enum`/`trait`/`type`/
  `const`/`static`/`mod`/`macro_rules!`);
- `use` imports (first-segment substitution; glob imports are opaque)
  and `pub use` re-exports (one hop each, chains followed while proven,
  cycles cut).

If the seeded path is outside the crate (`std::`, dependencies) or
resolves to no definition, the engine must fail unless the hunk
explicitly allows fallback.

Fallback directive:

```text
# fallback=syntax
```

A bare `# symbol` hunk verifies: every minus-side seed must resolve to
a definition in the indexed crate, and each matched call/qualified
site's spelled path must resolve to that same definition — sites that
spell a different definition with the same text (a vendored submodule
copy, a shadowing import) do not match and are never rewritten.
Resolution also proves short spellings: a bare `old_api(...)` site with
`use crate::old_api;` matches `# symbol Old=crate::old_api`. Plus-only
seeds (`New` in `Old` → `New`) are never required to resolve — they name
the rewrite target, which the crate may not define yet. With
`# fallback=syntax` the engine matches exact full-path text with no
resolution (byte-identical to the old behavior): a bare `old_api(...)`
call site does not match `# symbol Old=crate::old_api`. The report
carries a per-rule `symbol` key (`"resolved"` when the hook verified,
`"syntax"` when fallback carried the hunk; absent on non-symbol hunks).
For plain renames prefer a concrete path pattern (it is also the
canonical spelling: tree-aware comparison per D.5 normalizes fallback
symbol hunks to it) (`- crate::old_api(E)` / `+ crate::new_api(E)`),
which matches identically with fewer directives; raising never emits
`# symbol` (SPEC D.4). `# symbol` outside call shapes, the general
symbol positions (whole-path: `let`-init callee, general-expr callee,
bare or nested whole-path type; single-leading-prefix: `Old::suffix`,
`Old::<T>::suffix` with concrete suffix idents and structural suffix
args), field-type evolution (the same whole-path/nested/prefix law over
the field type: `x: Old`, `x: Vec<Old>`, `x: Old::Assoc`), and
use-prefix renames (whole-prefix `use Old` or single-leading
`use Old::sub`, remaining segments concrete) is rejected at C14
(SPEC 18.6), as is a symbol in a non-admitted slot (suffix segment,
abutting `Old<T>` args, `qself` tail, method ident, pattern interior,
field/label slot, statement sequence, non-leading `use` segment);
variant-payload, assoc-evolution, alias/const-evolution, macro
interiors, and `qself` tails ride concrete paths until a later slice.
The crate-local index also covers inherent and
trait `impl` methods as `crate::Type::method` canons, so
associated-function prefixes (`Old::make` with `# symbol
Old=crate::Old`) resolve through imports and short spellings exactly
like free functions; external bases (`Vec`, `Box`), generic self
types, and `qself` tails never index.
A lone `# fallback=syntax` with no `# symbol` is rejected: it constrains nothing (matching is already syntactic without resolution) — except on a `# follow=uses` hunk, where it is the WI-6a skip switch (Section 14.8).

A sole symbol ident carrying explicit turbofish (`Old::<u8>(E)` with `# symbol Old=crate::old_api`) expands to the seeded path and binds the turbofish args structurally (SPEC 18.6 R3); a placeholder hole (`Old::<T>(E)` with `# type T`) binds likewise, and tree-aware comparison per D.5 diet-renames the declared hole to the inferred `T` spelling. The same structural rule covers the stem of a prefix (`Old::<T>::make` binds `T` against the site's turbofish) and turbofish on a whole-path stem in general positions (full spellings match as well as short ones; an arg-free stem never matches a turbofish site and vice versa). Other symbol spellings keep exact-path behavior. A use-prefix head
symbol (`use Old`, `use Old::sub` with `# symbol Old=crate::old`)
expands to the seed on both postures: fallback matches the expanded
head segment-wise (group members included); resolved proves the target
head through the crate graph (`resolve_call` on the head must equal the
seed canon plus the concrete suffix, so re-export spellings match and
vendored twins miss). The replacement is the plus seed text plus the
plus suffix with the target tail preserved; a group boundary strictly
inside the match keeps the edit to the member portion when the outer
prefix is literally preserved, else the member is skipped.
> **Implementation note:** dependency/`std::` paths, glob-imported names, and macro-defined items are unresolvable by construction (stated, never guessed). No rust-analyzer / cargo-metadata integration: syntactic anchors plus this crate-local graph buy the provable majority of the trust at text cost.

---

## 20. Moving Code

Moving code is one of the main motivations for RSmPL-Git.

A normal Git patch must repeat the removed and inserted code. RSmPL can capture an AST item and reinsert it elsewhere.

### 20.1 Cut item

```diff
diff --git src/old.rs
@@ -20,12 @@ rule=cut_helper
# item I
# bind=Helper
- I
```

This means:

1. Match one Rust item inside `src/old.rs` lines `20..31`.
2. Bind that item as `Helper`.
3. Remove it from `src/old.rs`.

### 20.2 Paste item

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_helper
# use=Helper
+ Helper
```

This means:

1. Retrieve the captured item `Helper`.
2. Insert it at the target position near `src/new.rs` line `5`.

Multiple `# use=` pastes at exactly the same anchor apply in document
order: the first paste hunk in the patch appears first (topmost) in the
final file (Section 23.5). Same-anchor capture pastes are joined with
exactly one blank line between items; a lone paste ends with a single
`\n` and gains no leading/trailing blanks. Ordinary literal insertions
at the same anchor keep document order too but are never blank-joined —
the author controls their blanks explicitly.

An evolution paste carries a single-line header template instead
of the bare name (WI-8a `fn` items, WI-8 slice 1 `struct` items,
WI-8 slice 2 `enum` items, WI-8 slice 3 `union` items,
WI-8 slice 4 `type` items, WI-8 slice 5 `const` items,
WI-8 slice 6 `static` items, WI-8 slice 7 `mod` items,
WI-8 slice 8 `trait` items, WI-8 slice 9 signature-evolving `fn`
items):

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_helper
# use=Helper
+ pub fn renamed(...) { ... }
```

A signature-evolving `fn` paste (WI-8 slice 9) spells the plus-side
header concretely instead of `(...)`:

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_helper
# use=Helper
+ pub fn renamed(x: i64) -> i64 { ... }
```

A body-evolving `fn` paste (WI-6 slice 3b) renames the moved item and
its self-references together:

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_helper
# use=Helper
+ pub fn oldname as newname(...) { ... }
```

The `as` names the captured ident; the engine splices the header to
the new name and renames the captured body's self-recursive calls to
it. Occurrence law: only the header ident and self-recursive call/path
occurrences rewrite (strings, chars, comments, and lifetimes ride
byte-verbatim). A shadowing local (or its uses), a field/method name,
a macro name, or an attribute interior never rewrites — the paste
fails loud with `RSMPL_PATTERN_CONFLICT` naming the kind and the
captured line instead of guessing.

A container body-evolving paste (WI-19 slice 4) generalises the `as`
spelling per kind when a `struct`/`enum`/`union` move renames the item
and its body delta is exactly the container rename (a self-referential
field type or variant payload):

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_config
# use=Config
+ pub struct OldName as NewName { ... }
```

(`enum OldName as NewName { ... }` and `union OldName as NewName { ... }`
likewise.) The engine splices the header to the new name and renames
the captured suffix/body's type-path self-references under the same
occurrence law, generalised per kind: field idents, variant idents,
macro names, and attribute interiors never rewrite (loud veto naming
kind and line); vis, generics, and attrs must already be token-equal
(name-only scope — vis or signature deltas stay ordinary).

An inherent-`impl`-block body-evolving paste (WI-19 residue)
generalises the `as` spelling to the self-ty when an `impl Old { ... }`
move renames the self-ty and its body delta is exactly that rename
(self-paths `Old::…`/`Old` inside the block):

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_impl
# use=ImplBlock
+ impl OldName as NewName { ... }
```

The engine splices the header to the new self-ty and renames the
captured suffix/body's type-path self-references under the same
occurrence law: assoc-item names (`fn`/`const`/`type` named `Old`),
macro names, attribute interiors, and shadowed locals never rewrite
(loud veto naming kind and line); trait-impls (`impl Tr for Ty`) never
enter this form (WI-3 owns that header).

A signature+body joint `fn` paste (WI-19 slice 3) composes the
concrete-signature and `as` forms when a move renames the item, evolves
its signature in exactly one slot, and renames its self-references:

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_helper
# use=Helper
+ pub fn oldname as newname(x: i64) -> i64 { ... }
```

The template carries the new signature verbatim (single- or multi-line,
like the concrete-signature form) and the `as` rename; the engine
replaces the captured header span with the new-signature header and
renames the captured body's self-references under the same occurrence
law and the same loud vetoes as the `as` form. The joint fires in
raising only for exactly one signature-slot delta (generics params,
`where`, inputs+variadic, return — vis/name/qualifiers ride verbatim
for free); zero slots belongs to the `as` form and two or more slots
stay ordinary.

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_config
# use=Config
+ pub struct Renamed { ... }
```

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_kind
# use=Kind
+ pub enum Renamed { ... }
```

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_union
# use=Union
+ pub union Renamed { ... }
```

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_alias
# use=Alias
+ pub type Renamed = ...;
```

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_const
# use=Limit
+ pub const Renamed = ...;
```

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_static
# use=Counter
+ pub static mut Renamed = ...;
```

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_mod
# use=Helpers
+ pub mod renamed { ... }
```

```diff
diff --git src/new.rs
@@ -5,0 @@ rule=paste_trait
# use=Drawable
+ pub trait Renamed { ... }
```

The engine parses the captured item (which must match the template
kind: `fn` template needs a captured `fn`, `struct` template a
captured `struct`, `enum` template a captured `enum`, `union` template
a captured `union`, `type` template a captured `type`, `const` template
a captured `const`, `static` template a captured `static`, `mod` template
a captured `mod`, `trait` template a captured `trait`), replaces
exactly the header line's vis + `fn <Old>` / `struct <Old>` /
`enum <Old>` / `union <Old>` / `type <Old>` / `const <Old>` /
`static [mut ]<Old>` / `mod <Old>` / `trait <Old>` prefix with the template's verbatim prefix
(plus the template's `mut`, if present) and new
name, and keeps every other line byte-verbatim (multi-line params,
attrs, docs, body for the `(...)` `fn` form; the whole header span for
the concrete-signature `fn` form — the template carries the new
params, return, generics, and `where` verbatim while the body lines
ride; generics, where-clauses, and fields
for `struct`; generics, where-clauses, and variants for `enum`;
generics, where-clauses, and fields for `union`;
generics, where-clauses, and the aliased type for `type`;
the const type and initializer expr for `const`;
the static type and initializer expr for `static`, with `mut`
riding verbatim in the template like `fn` qualifiers ride the prefix;
empty inline (`{}`) or `;` declaration for `mod` (nested items veto —
the scalar `- I` cut cannot isolate them), with `unsafe` riding verbatim in the prefix like `fn` qualifiers;
empty traits only for `trait` (any trait item vetoes — the scalar `- I`
cut cannot isolate them, same narrowing as `mod`; `auto trait` and
`impl Trait for` restrictions veto — they stay ordinary), with `unsafe`
riding verbatim in the prefix like `fn` qualifiers).
A concrete-signature `fn` template requires a block-opening capture
(the header span runs from the `fn <Old>` line through the first line
ending with `{`, with body lines after it): single-line-header and
multi-line-header captures splice alike (the span replaces the span),
while single-line items fail loud (`RSMPL_PATTERN_CONFLICT`) — the
splice would drop the body.
The `fn` prefix is any valid Rust visibility + qualifier
combination, including restricted visibilities such as
`pub(crate)` / `pub(in path)`; the `struct`/`enum`/`union`/`type`/`const`/`static`/`mod`/`trait` prefix is any
valid Rust visibility (the `static` template additionally carries an
optional `mut` between the keyword and the ident; the `mod`/`trait` prefix may
additionally carry `unsafe` before the keyword). The `struct`/`enum`/`union`/`mod`/`trait` template's `{ ... }`
tail is a shape marker only (like the `fn` template's `(...)`):
the splice keeps the captured suffix (generics, fields, `;` for
tuple/unit structs; generics, variants for enums; generics, fields for
unions; `;` or `{}` for mods — declaration or empty inline only, nested items veto; `{}` for traits — empty traits only, any trait item vetoes) verbatim, and the `type`/`const`/`static` template's `= ...;` tail likewise
keeps generics plus `= ty;` / `: ty = expr;` verbatim, so all
struct shapes, all enum shapes, all union shapes, all alias shapes, all
const shapes, all static shapes, both mod shapes, and all trait shapes round-trip. A single-line plus
that looks like an `fn`/`struct`/`enum`/`union`/`type`/`const`/`static`/`mod`/`trait` header but is not a valid
template (no `(...)` or no `...` for `fn`; no `...` for `struct` /
`enum` / `union` / `mod` / `trait`; no `=` + `...` for `type` / `const` / `static`) fails loud with `RSMPL_PATTERN_CONFLICT`; a multi-line
plus with `# use=` fails loud with `RSMPL_CAPTURE_CONFLICT` —
captures are never silently ignored.

### 20.3 Capture naming and forward references

The value of `# bind=Helper` creates a named capture.

The value of `# use=Helper` imports that capture.

Names are patch-local.

A bare `# bind=<Name>` is a scalar capture: it binds exactly one AST object. Combined with `# allow=many` on an item-capture cut hunk (`- I`), it binds an ordered collection: every item matched in scope, in source order (ranges cite the original pre-patch file, never rebased). There is no separate syntax such as `# bind_list=Name` or `# use_list=Name` — the paste side is unchanged (`+ Name` splices the whole collection).

> **v0.1 implementation note:** `# bind=` is additionally restricted to item-capture cut hunks (a bare `- I` pattern), and `# use=` to pure insertion hunks (the bare `+ Name` verbatim paste, the evolution paste: single-line `+ <prefix>fn <New>(...) { ... }` / `+ <prefix>fn <New>(concrete signature) { ... }` (which may span lines when the plus header does) / `+ <prefix>fn <Old> as <New>(...) { ... }` (self-reference body rename) / `+ <prefix>fn <Old> as <New>(concrete signature) { ... }` (signature+body joint, which may span lines when the plus header does) / `+ <prefix>struct <New> { ... }` / `+ <prefix>enum <New> { ... }` / `+ <prefix>union <New> { ... }` / `+ <prefix>type <New> = ...;` / `+ <prefix>const <New> = ...;` / `+ <prefix>static [mut ]<New> = ...;` / `+ <prefix>mod <New> { ... }` / `+ <prefix>trait <New> { ... }` / `+ impl <Old> as <New> { ... }` (inherent-`impl` self-ty body rename)). Rewriting hunks (rename, call rewrite) cannot define captures, and non-insertion hunks cannot consume them. Captured text is the exact matched item span; paste validity at the insertion site is enforced by re-parsing the rewritten file. An evolution paste is verbatim plus-side text, never a match pattern: its idents (`T`, `Vec`, …) are not metavariable holes, so the unbound-plus gate never fires for it.

A hunk that declares both `# bind=<Name>` and `# allow=many` is a collection cut only on an item-capture cut hunk (`- I`); on any other shape it is invalid (`RSMPL_CAPTURE_CONFLICT`, same envelope as the shape fences below). Authors who need to move a single item keep the scalar form (separate scalar captures under distinct names).

An ordered collection binds every item matched in scope, in source order (ranges cite the original pre-patch file, never rebased). A paste hunk (`+ Name`) splices all items at its anchor in source order with the Section 20.2 one-blank join; the same collection may be pasted more than once (a copy — scalar fan-out parity). `# optional` with zero matches binds the empty collection, and the paste splices nothing. An evolution paste with a collection capture carries an `N`-hole in the name position (`N`, `new_N`, `N_new`, `pre_N_post` — `N` as a standalone underscore-separated segment, exactly once; `New` stays concrete): each item replays through the template with `N` bound to that item's old ident (`new = pre + old + post`), bodies/attrs/slots replay per item as scalar evolution pastes do. A concrete hole-less template on a collection fails loud with `RSMPL_CAPTURE_CONFLICT` (would rename many items to one name); kind-changing templates fail loud in the scalar splice. A collection cut deletes every matched item span; an overlapping ordinary hunk on the same lines fails the patch whole, never partially. A source file emptied to zero bytes is a first-class move source: its minus hunk covering the whole file (BOF..EOF) is sound for multi-item cuts (no outside preceding/following line to double-claim; `coalesce_deletes` union-merges the overlapping pure deletes).

The five shape violations fail fast with `RSMPL_CAPTURE_CONFLICT` (all exit 2; all carry `rule:`/`file:`; text omits the null `range:`/`suggestion:` lines, JSON carries explicit `range: null` / `suggestion: null`; suggestion is null throughout — contrast the missing-capture fences in Section 25.6, which set `suggestion:`):

```text
RSMPL_CAPTURE_CONFLICT
rule: r
file: src/lib.rs
reason: only one `# bind=<Name>` per hunk is supported in v0.1
```

```text
RSMPL_CAPTURE_CONFLICT
rule: r
file: src/lib.rs
reason: v0.1 `# bind=` is only supported on item-capture cut hunks (`- I`)
```

```text
RSMPL_CAPTURE_CONFLICT
rule: r
file: src/lib.rs
reason: v0.1 `# use=` is only supported on pure insertion hunks (`+ Name`)
```

```text
RSMPL_CAPTURE_CONFLICT
rule: r
file: src/lib.rs
reason: duplicate `# bind=Helper` (capture names must be unique)
```

```text
RSMPL_CAPTURE_CONFLICT
rule: r
file: src/lib.rs
reason: hunk both binds and uses `H` (self-cycle)
```

In the duplicate fence `Helper` echoes the offending `# bind=` name; in the
self-cycle fence `` `H` `` echoes the hunk's own `# bind=`/`# use=` name.


Unlike v0.4, captures are not limited to later hunks in document order. Implementations must support **forward capture references** within the same patch file. A `# use=Helper` hunk may appear before the hunk that declares `# bind=Helper`.

This requirement is necessary because standard Git-style patches usually order hunks by ascending line number. Moving an item upward in the same file naturally places the insertion hunk before the deletion hunk:

```diff
diff --git src/lib.rs
@@ -10,0 @@ rule=paste_helper_near_top
# use=Helper
+ Helper

@@ -100,12 @@ rule=cut_helper_from_bottom
# item I
# bind=Helper
- I
```

This patch is valid. The engine must resolve `Helper` by first collecting all capture definitions in the patch file, then evaluating the matching cut hunk against the original source coordinate system before materializing the paste.

### 20.4 Move validation

The engine must verify that:

- every `# use=<Name>` has exactly one corresponding `# bind=<Name>` (scalar or collection);
- `# bind=<Name>` + `# allow=many` appears only on item-capture cut hunks;
- forward references are resolved before application, not treated as missing captures;
- the cut hunk matched exactly one item;
- the pasted item is syntactically valid at the insertion site;
 - a moved item is not simultaneously deleted, replaced, or moved by another incompatible hunk;
 - imports, visibility, and module paths are not silently fixed unless explicitly requested;
 - same-anchor multi-pastes read in document order with one blank line
   between items (Section 20.2); mixing capture pastes and ordinary
   literal insertions at one anchor keeps document order, with the
   blank-join applying across the paste subsequence only.

If a capture dependency graph contains a cycle, the implementation must reject the patch unless the cycle consists only of independent references that do not require materialized output from one another.

### 20.5 Move plus import edit

RSmPL can move the code and normal Git hunks can adjust imports.

Example:

```diff
diff --git src/old.rs
@@ -20,12 @@ rule=cut_helper
# item I
# bind=Helper
- I

diff --git src/new.rs
@@ -5,0 @@ rule=paste_helper
# use=Helper
+ Helper

@@ -1,3 +1,4 @@
 use crate::common;
+use crate::old_dependency;
 use crate::other;
 fn main() {}
```

---

## 21. Macros

Rust macros complicate AST matching.

### 21.1 Macro modes

Supported macro modes:

| Directive | Meaning |
|---|---|
| `# macro=invocation` | Match macro calls as AST nodes. Default. |
| `# macro=token_tree` | Match inside macro token trees. |
| `# macro=expanded` | Match macro-expanded output. Requires build integration. |

### 21.2 Macro invocation rewrite

```diff
@@ -30,6 @@ rule=rename_macro
# macro M
# token_tree TT
- M!(TT)
+ new_macro!(TT)
```

### 21.3 Token-tree rewrite

```diff
@@ -30,1 @@ rule=rename_macro_body
# macro=token_tree
- old_mac!(...)
+ new_mac!(...)
```

A bare `...` body preserves the body verbatim (rename-only). Since
WI-7a the body may also be a top-level comma-separated argument list
in which each argument is whole — `...`, one bare metavariable, or
concrete token soup:

```diff
@@ -30,1 @@ rule=wrap_second_arg
# macro=token_tree
# allow=many
# expr E
- assert_eq!(E, TEXT_SECTION)
+ assert_eq!(E, TEXT_STRIPPED)
```

Per-position matching reuses invocation-mode semantics (SPEC 21.2):
a bare argument binds the whole argument, a concrete argument compares
token-equal (whitespace-insensitive), `...` absorbs any argument count
with backtracking, and repeated metavariables must match structurally
(SPEC 15.7). Plus templates splice per argument (`+ ...` replays the
whole body); the plus pattern's delimiter decides the emitted spelling
(`![...]` stays `![...]`, never normalized to parens). A declared hole
*inside* a larger argument (`old!(foo(E))`) never binds — it fails
fast with the verbatim-body diagnostic instead of matching literally.
Uppercase single-side idents read as unbound metavariables, exactly as
in every other shape (diet inference only binds shared both-sides
macro args to `expr`).

Composition (updated WI-7a, T133): invocation mode already binds
metavariables where the body is well-defined — comma-separated args
(`- assert_eq!(E, E)` / `+ assert_equal_once(E)`, with repeated-metavar
equality) and the lone whole-body capture (`- M!(TT)` / `+ new_macro!(TT)`
with `# macro M` + `# token_tree TT`). `# macro=token_tree` adds the
same per-argument positions for token soup the author scopes by name
and delimiter instead of by AST: one hole per top-level argument, no
descent into nested macro tokens (a nested macro inside a *bound*
argument position never matches — concrete positions compare
textually and never descend). Anything else nested in
a macro body goes through ordinary hunks — the same one-tool composition
as §14.9 (no new flags or directives; binding *inside* a token-tree
argument is rejected, not deferred). `macro_rules!` definitions are
items (Section 14.8), never token-tree invocation patterns: a
`- macro_rules! Old` / `+ macro_rules! New` pair renames the definition
ident, while `- Old!(...)` / `+ New!(...)` renames invocations.
Since WI-17b the engine additionally mints invocation candidates one
textual level inside `macro_rules!` bodies (`syn::visit` never enters
the token stream, so a string/comment-aware scan over the definition
slice provides them): `- inner!(1)` / `+ inner!(2)` matches the two
arm-body sites structurally, with the same per-argument semantics as
above. Nested-inside-nested inners are minted but never emitted
(depth-2 stays ordinary).

Engine-verbatim shape rejections (both exit 2; both carry `rule:`/`file:`; text omits the null `range:`/`suggestion:` lines, JSON carries explicit nulls):

```text
RSMPL_PATTERN_CONFLICT
rule: m
file: a.rs
reason: v0.1 macro patterns must have `NAME!(...)` shape, got `foo(E)`
```

```text
RSMPL_PATTERN_CONFLICT
rule: m
file: a.rs
reason: v0.1 `# macro=token_tree` supports only `NAME!(...)` with verbatim body
```

The first `got ...` tail echoes the offending minus text verbatim (here `foo(E)`); the second fires when a declared hole sits inside a larger token-tree argument (`old!(foo(E))`) — whole-argument holes (`old!(E)`) and bare `...` bodies match instead. The `expected macro pattern` fallthrough arm is dispatch-unreachable (the planner routes only macro-classified patterns there) and carries no fence.

### 21.4 Ellipsis in token-tree mode

In `# macro=token_tree` mode, `...` is a token-tree wildcard. It matches any sequence of zero or more tokens inside the current token-tree delimiter, subject to balanced delimiter preservation.

Example:

```diff
@@ -30,1 @@ rule=wrap_macro_body
# macro=token_tree
- mac!(...)
+ mac_wrapped!(...)
```

This matches invocations such as:

```rust
mac!()
```

```rust
mac!(a, b, c)
```

```rust
mac!({ let x = 1; x }, nested!(y))
```

The wildcard captures the token sequence between the macro delimiters. If reused in a `+` pattern, the captured tokens are pasted as the same sequence:

```diff
@@ -30,1 @@ rule=rename_macro_preserve_body
# macro=token_tree
- mac!(...)
+ new_mac!(...)
```

In token-tree mode, `...` is token-based, not AST-based. It does not enforce Rust expression, statement, item, or list boundaries unless the implementation explicitly parses the captured token sequence in a narrower Rust context.

If a token-tree pattern contains multiple `...` wildcards at the same delimiter depth, the engine must either use a documented deterministic left-to-right matching strategy or reject the pattern as ambiguous. Rejection is recommended for v0.1 unless the implementation documents exact behavior.

> **v0.1 implementation note:** multiple `...` wildcards (argument lists, path segments, macro bodies) match with a documented left-to-right greedy strategy with backtracking (earliest split first, later splits on tail failure); `foo(...)` absorbs any argument count including zero.

### 21.5 Expanded macro rewrite

```text
# macro=expanded
```

This mode requires the engine to know macro expansion output and map expanded spans back to source spans.

The reference engine has no such mapping in v0.1 and fails fast with
`RSMPL_PATTERN_CONFLICT` (rule + file lines; text omits the null
range/suggestion lines, JSON carries explicit nulls):

```text
RSMPL_PATTERN_CONFLICT
rule: rename_macro
file: src/lib.rs
reason: `# macro=expanded` requires build integration (macro expansion span mapping), unavailable in v0.1
```

---

## 22. Formatting and Trivia Preservation

### 22.1 Recommended default

> **v0.1 implementation note:** only `# format=preserve` is implemented. Non-`preserve` values are rejected fail-fast with a diagnostic pointing at `cargo fmt` instead of being silently ignored. Post-apply `cargo fmt` is the permanent answer for formatting (same composition as Section 21.3 for macro bodies): an in-tool formatting pass is rejected, not deferred, so no new flags or directives are needed.

The recommended default is:

```text
# format=preserve
```

The engine should preserve:

- indentation;
- comments;
- blank lines;
- attributes;
- original ordering;
- unaffected source text.

### 22.2 Rustfmt mode

```text
# format=rustfmt
```

is rejected in v0.1: the tool never shells out to a formatter. Run
`cargo fmt` after apply instead — one cheap command covers every case
(evolved signatures, rewritten nodes) with no availability or version-drift
failure modes inside the binary. `# format_scope=*` is likewise rejected
(the parser already points at removing the directive).

 Verbatim (rule + file lines; text omits the null range line, JSON
 explicit nulls; the suggestion rides in both modes):

 ```text
 RSMPL_PATTERN_CONFLICT
 rule: r
 file: src/lib.rs
 reason: only `# format=preserve` is supported in v0.1
 suggestion: remove the directive and run `cargo fmt` after apply
 ```

 The same reason covers `# format=canonical` (Section 22.3).

### 22.3 Canonical mode

```text
# format=canonical
```

is rejected in v0.1 for the same reason: run `cargo fmt` after apply. An
engine pretty-printer stays out of scope — prototypes do not need a second
formatting dialect, and human-reviewed patches want `cargo fmt` output.

---

## 23. Application Model and Ordering

RSmPL-Git uses a **planning model** rather than a naive immediate sequential application model for semantic hunks.

The engine must read the whole patch file, build a patch plan, resolve capture dependencies, detect conflicts, and then materialize file changes.

Document order remains meaningful for human review and for tie-breaking where explicitly specified, but it is not the coordinate system for RSmPL line ranges and it is not a restriction on capture references.

### 23.1 Required processing phases

A conforming implementation should process a patch using these phases:

1. Parse the complete patch file.
2. Group hunks by target file.
3. Parse each original target file and record original AST spans.
4. Collect all RSmPL directives, metavariable declarations, `# bind` declarations, and `# use` references.
5. Resolve the capture dependency graph, including forward references.
6. Match every RSmPL hunk against original-source ranges.
7. Convert RSmPL transformations and ordinary hunks into an edit plan over original source coordinates.
8. Reject incompatible overlapping edits.
9. Materialize the final file contents.
10. Optionally run requested formatting.
11. Verify the resulting Rust parses.
12. Emit a concrete ordinary Git diff if requested.

Implementations may use a different internal algorithm if the externally visible behavior is equivalent.

A failure at any phase aborts the whole patch: matching, planning, and lowering are all-or-nothing across file sections — a match failure in one file section discards successful matches in the others. No file is written and nothing is applied (exit 2, worktree untouched in both `--check` and apply modes); the error envelope names the failing hunk (`rule` + `file`, e.g. Section 25.4).

### 23.2 Forward capture references are required

A capture in any hunk may be used by any other hunk in the same patch file, regardless of document order.

This is valid:

```diff
diff --git src/b.rs
...
@@ -20,0 @@ rule=paste
# use=Thing
+ Thing

diff --git src/a.rs
...
@@ -10,5 @@ rule=cut
# item I
# bind=Thing
- I
```

The engine must resolve `Thing` by evaluating the `# bind=Thing` hunk before materializing the `# use=Thing` insertion, even though the `# use` appears first in the patch file.

### 23.3 Upward moves in the same file

Forward capture references are especially important for moving code upward in a file.

A Git-like patch ordered by line number places the insertion near the top before the deletion near the bottom:

```diff
diff --git src/lib.rs
@@ -10,0 @@ rule=paste_helper
# use=Helper
+ Helper

@@ -100,12 @@ rule=cut_helper
# item I
# bind=Helper
- I
```

This patch is valid and must be accepted if both hunks match and no conflict is detected.

An implementation must not reject the paste hunk merely because `# bind=Helper` appears later in the document.

### 23.4 Line ranges are not rebased after earlier hunks

For RSmPL hunks, old-side hunk ranges always refer to the original pre-application file.

The following must not happen:

```text
apply hunk 1
count how many lines hunk 1 inserted or removed
shift hunk 2's RSmPL search range by that amount
```

That behavior is forbidden for RSmPL hunks.

Instead, hunk 2 must be matched against the original AST spans for the range it declared.

This remains true regardless of output line-count drift: a rewrite may
produce a different number of lines than the input (in v0.1
`# format=rustfmt` is rejected, so any reformatting happens via post-apply
`cargo fmt` — later hunks still cite original lines).

### 23.5 Interaction with ordinary hunks

Ordinary hunks retain normal unified-diff meaning, but an RSmPL-aware engine should treat them as edit operations against the original source coordinate system when combining them with semantic hunks.

If an ordinary hunk and an RSmPL hunk target disjoint original source ranges, they may be combined normally.

If they overlap, the engine must reject the patch unless it can prove that the ordinary hunk is a textual refinement of the same semantic transformation.

Engine-verbatim overlap rejection (exit 2):

```text
GIT_HUNK_CONFLICT
file: src/lib.rs
reason: overlapping edits cannot be merged
suggestion: apply the overlapping rewrites in separate runs, or make the ranges disjoint
```

Text carries `file:` but no `rule:`/`old_range:` lines (JSON: `rule: null` / `range: null`, file set, same `reason`/`suggestion`). The canonical fence plus the contrast with the ordinary-hunk content conflict lives in Section 25.7.

Document order is the deterministic tie-breaker for non-overlapping insertions at exactly the same original insertion point: the first insertion hunk in the patch appears first (topmost) in the final file. Same-anchor capture pastes additionally gain one blank line between items (Section 20.2).

> **v0.1 implementation note:** multiple insertions at exactly the same point are applied in document order (first hunk topmost); an insertion strictly inside a deleted or replaced range is rejected as an overlap conflict.
Overlapping pure deletions union-merge instead (T464): two edits that both only delete can never conflict, so adjacent item deletes (which double-claim their shared separator blank) apply as the union. Touching-only pairs (`[a,b)` + `[b,c)`) skip the merge when an insertion anchors exactly at the seam point `b` (WI-9f) — the insertion composes at the boundary, and merging first would falsely place it strictly inside the union.

---

## 24. Validation Rules

An implementation must reject an RSmPL hunk if any required validation fails.

### 24.1 Structural validation

Reject if:

1. hunk header cannot be parsed;
2. a `# lang=` directive names a language other than `rust`;
3. directive syntax is invalid;
4. metavariable declarations are malformed;
5. literal-token quoting is malformed (engine-verbatim reasons + envelopes in Section 15.4);
6. the target file cannot be determined;
7. the old-side range is invalid for the original source file state.

### 24.2 Rust parsing validation

Reject if:

1. the target file cannot be parsed as Rust in `mode=ast`;
2. the RSmPL pattern cannot be parsed;
3. the resulting file cannot be parsed after rewrite.

### 24.3 Match validation

Reject if:

1. zero matches are found and `# optional` is not set;
2. multiple matches are found and `# allow=many` is not set;
3. a metavariable is used before binding;
4. repeated occurrences of the same metavariable are not structurally equal within a candidate match;
5. a semantic symbol cannot be resolved;
6. a match is outside the allowed scope.

### 24.4 Rewrite validation

Reject if:

1. inserted syntax is invalid at the target site;
2. captured syntax is inserted into an incompatible AST context;
3. a `# use` capture remains unresolved after full-patch forward-reference resolution;
4. a scalar `# bind` hunk matches more than one AST object, or a `# bind` + `# allow=many` hunk is not an item-capture cut;
5. a rewrite would produce duplicate or invalid syntax;
6. a move would remove a required node and fail to insert it.
7. a `# repeat` hunk is not a call/method-call/general-expression rewrite, is combined with `# bind=`/`# use=`, or has an empty minus/plus side.

### 24.5 Ordinary Git validation

Ordinary hunks must satisfy normal Git patch application rules.

### 24.6 Trust validation (report evidence)

Byte-identity outside the reported spans holds by construction: edits
are the only mutation (materialization is a pure function of original
plus the recorded edits), so no re-derivation is reported. Accept only
if:

1. every rewritten file re-parses as Rust (validation item 3,
   Section 24.2; ordinary-only files are trivially true — they carry
   no Rust parse); a failure is `RSMPL_PARSE_CONFLICT`, never a
   warning;
2. the sweep change gate holds: every non-`optional` file-scope
   `# allow=many` hunk that matched has at least one byte-changing
   definition edit. A sweep claims every site in scope — a sweep that
   changes nothing (e.g. `foo(E)` -> `foo(E)`) fails loud with
   `RSMPL_MATCH_CONFLICT` (`postcondition: sweep matched but changed
   no bytes ...`), never a silent empty lowering with a clean report.
   Deliberately not a "minus must be zero in the post-image" check:
   widening rewrites legitimately re-match their own output (`let P: T
   = E` -> `let P: T = E.wrapping_add(1)` still matches `let P: T = E`
   on the wrapped init). Ranged singles owe no gate (other sites may
   legitimately remain; byte-identical fixed-point instantiations per
   Section 18.6 R4 `P @` stay valid matches), `optional` zero-edit
   hunks are skipped by design, and insertions/cuts never participate.
 3. the plus-pattern gate holds (WI-1b, WI-1.1, WI-1.2, WI-1.4, WI-1.5):
     every rewrite hunk with at least one match has its plus template
     compiled to a pattern with the same compiler and re-matched at the
     reported spans in the post-image (backticks stripped, `$`-sigils
     resolved; the post-image is parsed once per file and shared by all
     rules over that file, with one line index per file shared the same
     way). Rename kinds (`fn`, item, field, variant, use) and
     impl-context method renames verify span-locally by syn ident
     equality (WI-1.4): at least `matches` reported spans must overlap
     the new ident's own token lines (`fn {name}` for impl-context)
      with exact text equality — occurrence elsewhere in the file never
      counts, a longer ident merely containing the needle (`New2`) never
      counts, and a span still carrying the old ident fails.
      Follow spans (WI-6) verify the same way at their own spans: every
      `follow`-labeled use span in the post-image must name the new ident
      as an exact ident token (same boundary discipline, so the
      shorthand `S { new: old }` verifies by its `new` token and a
      reverted `S { old }` fails). WI-6b tightens the probe to byte-exact:
      each follow edit's recorded `(start, end, replacement)` triple maps
      to a post-image byte offset (original offset plus the net delta of
      earlier edits) and the post slice there must equal the replacement
      with ident boundaries — a sibling `new` elsewhere on the same line
      never satisfies the span. Follow-only entries whose spans all
      verify report `instantiated`; any missing span fails loud with
      `RSMPL_MATCH_CONFLICT`, never a silent downgrade. Same-file follow
      spans verify identically alongside the definition probe.
      `Expr`/`General`/`Macro` verify by plus-pattern re-match (WI-1.5):
     the recorded plus side is compiled as a pattern (minus = plus,
     same directives, forced file-scope `# allow=many`) and re-run
     through the existing matchers over the post-image; every reported
     span must overlap a plus match whose bindings are SPEC-15.7-equal
     to the recorded ones (string-aware canonical equality:
     whitespace-insensitive outside string literals,
     trailing-comma-insensitive). When one span per recorded binding
     holds, the check is pairwise — binding `i` must verify at span
     `i`, so a swapped or reverted site fails even when its text occurs
     at a sibling span; otherwise each span must overlap a plus match
     equal to some recorded map (the single-map multi-edit case checks
     every span against the one map). All-empty bindings check overlap
     only (the concrete template must match at each span). Empty spans
     require at least `matches` plus matches file-wide (span-less
     callers keep the old posture, never a new veto). No instantiation,
     no text windows: overlap comes from matcher byte offsets via the
     post-image line index, and binding values compare structurally.
     A rewrite that did not land as authored (e.g. the post-image still
     names the old ident, or a span was reverted while the same text
     occurs elsewhere) fails loud with `RSMPL_MATCH_CONFLICT`
    (`postcondition: plus pattern did not re-match ...`), never a
     silent `verified`. Unverifiable plus shapes downgrade the rule to
     `reparse-only` (honest, never over-claimed): insertions, cuts,
     pastes, `Raw`, impl-context hunks whose plus-side `fn` name
     cannot be extracted, and `Expr`/`General`/`Macro` hunks whose plus
     side fails to compile as a pattern (or fails to re-match for an
     engine reason, or finds no attributable plus match) — including
     unparseable post-images, which never veto.
       A move pair (one item cut plus one paste of
       the same capture — scalar or `# allow=many` collection in
       source order — exactly one edit at the paste position) verifies
       as `postcondition` on both entries when the post-image holds the
       expected paste bytes at the paste edit's byte-exact post offset and,
       per captured item, no same-kind same-ident (old ident for evolving)
       item starts inside the cut spans translated to post-image coordinates
       (the same-file paste block itself is excluded from the absence scan,
       or every same-file move would fail on its own output).
       Bare pastes expect the joined captured item bytes (per-item
       `trim_end`, `"\n\n"` join, trailing `"\n"`); evolving pastes expect
       the template-replayed bytes (scalar splice, or per-item
       concretize+splice joined identically — kind-preserving, new ident
       `pre_old_post` per the `N`-hole). Empty collections, multi-pastes
       sharing one anchor, multi-inserts sharing one anchor, unnameable
       items (`impl` blocks and friends), hole-less collection
       templates, kind mismatches, standalone-unparseable insertions,
       and unparseable post-images stay `reparse-only`; in-range mismatch
       fails loud, never a silent verify.
        A pure insertion (`+ <text>` with no `# use=`, exactly one
        insertion edit at its anchor) verifies as `postcondition` when
        the post-image holds the inserted bytes at the byte-exact post
        offset, the inserted slice parses standalone as Rust (`Item` /
        `Stmt` / `Expr` / `Type` / multi-item `File`), and the file
        re-parses.
        A member addition (`FieldAddition` / `VariantAddition` /
        `ArmAddition` point-insertion, ranged or file-scope) verifies as
        `postcondition` when every recorded insert holds its bytes at the
        byte-exact post offset and the container holds the member in the
        re-parsing post-image (struct holds the field / enum holds the
        variant / a same-scrutinee `match` holds the arm).
        Joint rename+addition halves (a shared-rule item rename plus its addition) verify against the renamed (new) container.
        A member evolution (`FieldTypeEvolution` /
        `VariantPayloadEvolution` tuple / struct / discriminant, ranged
        or file-scope) verifies as `postcondition` when its plus-pattern
        re-match passed and the container holds the member in the
        re-parsing post-image (struct holds the field / enum holds the
        variant; discriminant content is proven by the plus re-match, so
        variant presence completes the verdict). A membership miss
        abstains to `instantiated` (the plus gate already fails loud on
        content mismatch; an unparseable or macro-hidden post-image
        never vetoes).
        An unpaired cut (a scalar or collection cut with no same-name
        paste — a deletion) verifies as `postcondition` when, per
        captured item, no same-kind same-ident item starts inside the
        cut spans translated to post-image coordinates and the file
        re-parses.
     Per-rule evidence is `semantics`: `instantiated` (plus-pattern ran,
      including verified follow-only entries), `postcondition` (sweep gate
      + plus-pattern both ran, or a verified move / insertion / cut /
      member-addition / member-evolution structural probe),
     `reparse-only` (unverified insertions, cuts, pastes,
    unverified follow-only extras, unverifiable plus, or a local-binder
    (`let`/closure-param/match-arm/for/`if let`/`while let`/fn-param) rename with leftover-use orphans
    (WI-2a/WI-2c/WI-2d)),
    `not-applicable` (`optional` zero-edit hunks).
     The global `checked.semantics` is `verified` iff every rule is
    `instantiated`/`postcondition`, no ordinary hunk is present, every
    rule's engine orphan count is zero, every rule's follow-skip count
    is zero, and no rule's evidence was truncated (orphan cap 8 /
    orphan text 80 chars / site bindings cap 8 / binding value 80
    chars all unhit — counts read before JSON truncation);
     otherwise `partial` (unverified insertions, cuts, pastes,
    member-additions, ordinary hunks, unverified
    follow-only extras, optional zero-edits, any orphans, any
    follow-skips, and any truncated evidence all force `partial`).
    Text mode prints one
    `incomplete: rule <name> (<file>): <n> orphans, <m> follow-skipped[, truncated]`
    line per blocking rule on stderr (JSON stays stdout-only).
    A definition-only rename with live uses, a follow rename with
    skipped sites, or a wide rename with truncated evidence therefore
    reports `partial` with its blocking counts named — never
    `verified`. This completeness clause is a breaking meaning change
    of `verified` (v5; the version bump lands in its own item, the
    derivation lands here).

---

## 25. Conflict and Error Model

The engine should report structured errors.

### 25.1 Range conflict

The declared range is invalid.

```text
RSMPL_RANGE_CONFLICT
rule: rename_calls
file: src/lib.rs
old_range: 40..48
reason: old range 40,8 exceeds file length (1 lines)
suggestion: read `src/lib.rs` to list valid ranges
```

The `old_range` is `start..start+len` (so `@@ -40,8 @@` reports
`40..48`), and the reason names the `@@` start/len and the file's line
count `N`. The JSON envelope carries the same reason with explicit
`range` (null fields are omitted from text output). The `suggestion`
names the file to read (repo-relative path) so an agent repairs without
guessing flags. The failure also carries a `fix_range` repair
(`file`/`file_lines`/`write_instead`, SPEC D.2): the largest in-file
range starting at the cited start, or the last line when the start is
past EOF — paste the header verbatim and retry.

### 25.2 Parse conflict

Rust parsing failed.

```text
RSMPL_PARSE_CONFLICT
file: src/lib.rs
reason: target file does not parse as Rust: cannot parse string into token stream
```

The `: ...` suffix is the underlying parser error and varies per file; the
failure carries no `rule:`/`old_range:`/`suggestion:` lines (null fields are
omitted from text output; the JSON envelope carries explicit `rule: null` /
`range: null` / `suggestion: null`). The Rust-parse gate fires only for files
carrying >=1 RSmPL hunk (Section 11): ordinary-only sections skip both the
original parse and the rewritten-file reparse, so non-Rust targets apply cleanly.

The rewritten-file reparse (validation item 3, Section 24.2) fails with `RSMPL_PARSE_CONFLICT` (exit 2; file-only envelope — text omits the null `rule:`/`range:`/`suggestion:` lines, JSON carries explicit nulls). Engine-verbatim (the `: {parser error}` suffix is the underlying parser error and varies per output):

```text
RSMPL_PARSE_CONFLICT
file: a.rs
reason: rewritten file does not parse as Rust: expected an expression
```

One trigger is a `$`-prefixed spelling in bare (or default) mode: `$E` is not a metavariable reference there, so the literal `$` survives into the output (Section 15.5).

A `diff --git a/old b/new` section whose stripped paths differ is a
file rename section (Section 6): it applies against the old file's
content and lowers back to `rename from`/`rename to` (never silently
writing to the b-side as a plain modify). The pre-T377 fail-fast
rejection (`file rename ... is not supported in v0.1` with the
`git mv`-first suggestion) is retired.

Three further structural rejections share the `RSMPL_PARSE_CONFLICT`
code (all exit 2). First, the RSmPL-only elision gate (Section 8):

```text
RSMPL_PARSE_CONFLICT
reason: ordinary hunk requires both -old,len +new,len ranges: @@ -1,1 @@
suggestion: write both ranges (e.g. `@@ -1,2 +1,2 @@`) or add a `#` directive line to make it an RSmPL hunk
```

Text omits the null `rule:`/`file:`/`range:` lines (JSON explicit
nulls); the reason echoes the offending header line. It also carries a
`fix_header` repair whose `expected_header` mirrors the old side (drop
it in verbatim and recount — counts are verified at apply).

Second, the malformed-header family (Section 8 documents the same
fences): `invalid hunk header (missing closing @@): {line}`,
`invalid hunk header ranges: {line}`, `invalid hunk range: {range}` —
each carries `old_range: line N` (the hunk-header line number) with null
`rule:`/`file:`/`suggestion:` lines omitted from text (explicit nulls
in JSON), plus a header-template `fix_pattern` repair
(`write the header as ...`, SPEC D.2):

```text
RSMPL_PARSE_CONFLICT
old_range: line 2
reason: invalid hunk header (missing closing @@): @@ -1,1 rule=r
```

```text
RSMPL_PARSE_CONFLICT
old_range: line 2
reason: invalid hunk header ranges: @@ -1,1 +1,1 +1,1 @@ rule=r
```

```text
RSMPL_PARSE_CONFLICT
old_range: line 2
reason: invalid hunk range: -foo
```

The bare `invalid hunk header: {line}` arm is defensive-only
(unreachable via the CLI — the header parser only runs on `@@`-led
lines; T287/T290/T291 precedent, documented not pinned).

Third, lines outside any file section — a hunk header, file metadata,
or anything else:

```text
RSMPL_PARSE_CONFLICT
reason: hunk header outside file section
```

```text
RSMPL_PARSE_CONFLICT
reason: file metadata outside file section: --- a/src/lib.rs
```

```text
RSMPL_PARSE_CONFLICT
reason: unexpected line outside file section: hello
```

All three carry a section-template `fix_pattern` repair beside the
reason (null `rule:`/`file:`/`range:`/`suggestion:` — omitted from text,
explicit nulls in JSON); `{line}` echoes the offending line verbatim.

Fourth, an empty patch (zero file sections):

An indented hunk header (T365) is rejected fail-fast: a header with
leading whitespace is a valid unified-diff context line, so without
this gate it would land in file metadata and lower to an empty no-op:

```text
RSMPL_PARSE_CONFLICT
file: src/lib.rs
old_range: line 4
reason: hunk header with leading whitespace (headers must start at column 0):  @@ -1,1 +1,1 @@
suggestion: remove the leading whitespace so the header starts at column 0
```

An ordinary hunk whose header counts disagree with the body (T367) is
rejected fail-fast: old-side lines (` ` + `-`) must equal `-old,len`
and new-side lines (` ` + `+`) must equal `+new,len` (`\ No newline`
markers are neither side; RSmPL hunks are exempt, Section 10.4):

```text
RSMPL_PARSE_CONFLICT
file: src/lib.rs
old_range: line 4
reason: ordinary hunk body has 3 old-side and 3 new-side lines but header claims -1,5 +1,3: @@ -1,5 +1,3 @@
suggestion: fix the header counts to match the body lines (old-side = ` ` + `-` lines, new-side = ` ` + `+` lines); if the counts look right, a line may be missing its 1-char marker (` ` context / `-` removal / `+` addition, exactly one marker char) so it never entered the body -- check marker indent before changing the header, then paste `expected_header` from the repair
```

When the body was truncated by an unprefixed line, the same code
names the suspect instead of offering a drop-in header (a header
computed from the truncated body would silently drop that line, so it
is never offered):

```text
RSMPL_PARSE_CONFLICT
file: src/lib.rs
old_range: line 4
reason: ordinary hunk body has 1 old-side and 1 new-side lines but header claims -1,3 +1,3: @@ -1,3 +1,3 @@; suspect line 6: `foo` never entered the body (missing 1-char marker?)
suggestion: line 6 never entered the body — it is missing its 1-char marker (` ` context / `-` removal / `+` addition, exactly one marker char); prefix line 6 with the marker so it enters the body, then recount (do not shrink the header past the dropped line)
```

Both carry file + header-line range + suggestion (no `rule:` line
unless the hunk header carries one; JSON explicit throughout), plus
the machine repair: the whitespace arm a header-template `fix_pattern`,
the pure-miscount arm the `fix_header` drop-in header (SPEC D.2), the
suspect arm `fix_header` with `expected_header: null` plus
`suspect_line` / `suspect_text` / `fix_line` (SPEC D.2). A body line
missing its 1-char prefix never enters the body, so marker-indent
surfaces as a count symptom: the count-arm suggestion names the
1-char marker rule before directing the agent to the repair. Hunk
headers, blank separators, file metadata, and EOF are never suspects:
a mismatch ending there is a pure miscount with a well-formed body.
The T431 trailing-blank repair runs before this gate, so a suspect is
only named when the body still mismatches after trailing blanks are
consumed.

```text
RSMPL_PARSE_CONFLICT
reason: empty patch: no `diff --git` file sections
suggestion: provide at least one `diff --git a/.. b/..` file section with a hunk (see SPEC 6 minimal recipe); check for an empty patch input (stdin, --file, or --patch)
```

Bare-plus-suggestion (null `rule:`/`file:`/`range:` omitted from text,
explicit nulls in JSON) plus an empty-template `fix_pattern` repair —
the same envelope as the Section 7.3
misplaced-directive fence (which likewise carries a `fix_pattern`
repair naming the move); the prose contract lives in Section 6.

### 25.3 Pattern conflict

The RSmPL pattern failed to parse. Unknown directives (`# foo`,
`# foo=bar`), `# edition=*`, and `# format_scope=*` are rejected here
(no silent ignore — verbatim fences in Section 9.6: three unknown-directive
shapes plus the two `unsupported directive` shapes, each with its suggestion note), as is any hunk-header `k=v` tag other
than `rule=` (no silent dead tags — verbatim reason + envelope, Section 8.3),
any `# root=` value other than `item/expr/stmt/type` (no silent heuristic
fallthrough — verbatim reason + envelope, Section 13.4),
any `# identifier/# path/# lifetime/# field/# attributes`
metavariable declaration (no kind-specific binding in v0.1 — verbatim
reasons in Section 15.1), any `# repeat=<value>`
(parse-time verbatim reason + suggestion, Section 9.6), any bad value
for a known directive key (other verbatim reasons in Section 9.6),
any `# repeat` on a
non-call/non-general-expression shape, on a capture hunk, or with an empty
minus/plus side (no silent no-op repeat, Sections 17.8, 24.4), any
item-rename hunk whose minus/plus qualifier prefixes differ, whose vis
step leaves the closed vis set, or whose vis step rides anything but a rename (no silent qualifier/vis change), any item-rename side with a
kind-illegal prefix (`unsafe` off `trait`/`impl`, visibility on `impl`),
or any kind mismatch, identical-name, or `auto trait` header — all Section
14.8, which lists the engine-verbatim reasons,
and any call/method-call hunk whose `+` side holds more `...` tokens
than the `-` side has argument ellipses to replay (verbatim reason +
suggestion, Section 16.3),
any `# mode=tokens` hunk outside a macro token-tree pattern, or any
hunk whose `+` side names an unbound metavariable (both fenced below),
any non-`NAME!(...)` macro pattern or non-verbatim `# macro=token_tree`
body (both fenced in Section 21.3),
or any malformed literal-token quote — `r#`` split (pattern or
directive), unbalanced backticks, empty/multi-token/invalid-raw/
invalid-literal quotes (verbatim reasons + envelopes, Section 15.4).
Also rejected with `RSMPL_PATTERN_CONFLICT` (rule + file lines; text omits null `range:`/`suggestion:` lines except the attrs-merge pair, which carries the set suggestion — JSON explicit throughout): any `use`-path pattern carrying grouped/glob tails, `as` tails, `!`, `...`, `$Meta`, backticks, or declared-metavariable segments (seven engine-verbatim reasons, Section 14.9); any unterminated fn signature whose tail swallows the body lines (three engine-verbatim needles, Section 14.5); and any `+` side whose attributes ambiguously merge with the `-` side (two engine-verbatim reasons with suggestion, Section 14.6); any fn-slot/param-list `...`/body misuse (five engine-verbatim reasons) or structural fn-pattern misuse (eleven engine-verbatim reasons, both Section 14.7); any concrete-shape, missing-name, bare-side-mismatch, or lexical-name item pattern (six engine-verbatim reasons, Section 14.8); any `use`-path tail misuse — mirrored attributes, invalid paths/segments, or a missing path (four engine-verbatim reasons, Section 14.9) — plus the two bare-side `use` fences in the same section; and any degenerate empty `-`/`+` pattern pair (the one reachable Raw shape, fenced below).

The body list above gains four final members: the Section 14.6 attribute match-time fences, the Section 13.4 trait-method visibility fence, the Section 16.5 `+`-path `...` fence, and the Section 14.7 structural fn-pattern fences. It gains three further members: the Section 9.6 declaration-name fences (`missing name`, `invalid name`, `empty path`, `empty capture`), the Section 16 standalone-`...` fence, and the Section 17.8 repeat-both fence. Match-time `invalid expression` / `invalid `let`` / `invalid type` re-parses are defensive-only: the classifier pre-validates the same pattern string at compile time (C-gates above), so a CLI hunk cannot reach them (T287/T290/T291 precedent, documented not pinned).
It gains three more members: the Section 9.6 empty-directive and second invalid-name fences, the Section 20.3 self-cycle fence, and the Section 25.2 outside-section twins plus the empty-patch fence. It gains thirteen further members: the Section 18.6 C-table compile-time fences — twelve live C1-C12 + C18 rejections (`...` in general shape, four `# root=` mismatches, empty-`+` expression, empty-`+` type, embedded `let` metavariables, `returns=none` on non-function, leading attributes on non-`let`, multi-statement minus, and the two attribute-merge rejections shared with Section 14.6) plus the WI-5a C2-closure fire (`let-else` minus now applies), each pinned text+JSON+untouched (fires pinned as success+output) in `general_compile_verbatim_e2e.rs` (C13/C14/C15 already pinned via their sections; C16 pinned in `root_item_verbatim_e2e.rs`; C18 pinned in the same file). It gains one further member: the WI-6a C17 misplaced-`# follow=uses` fence (rule+file envelope, set suggestion), pinned text+JSON+untouched in `follow_uses_e2e.rs`.

It gains one further member: the Section 14.8 item attr-merge fence (plus-side-omitted verbatim reason, rule+file envelope with null range/suggestion; the differ case reuses the 14.6 second fence byte-identical, suggestion set). Defensive-only, documented without pins (T287/T290/T291 precedent — no CLI trigger, so no fence to pin): the seven `expected X pattern` dispatch arms (`function`, `item capture`, `insertion`, `impl-context`, `expression`, `use-path`, `generalized stmt/expr/type` — plan.rs routes each `PatternKind` to its own matcher, cf. the 21.3 macro note), the `cannot analyze method signature` synth backstop (rendered from an already-parsed signature; no CLI trigger post-T190/T221), the `general statement patterns must be ...` arm alongside the `invalid let`/`expression`/`type` match-time re-parses the body list already names (the classifier pre-validates the same strings at compile time), and the span-guard family (`span line ... outside file`, `edit range ... outside file`, `... name span mismatch (file changed during matching?)`, `use prefix span is empty (file changed during matching?)` — the target file is immutable during matching).

It gains one further member: the Section 9.4 whitespace-in-name fence (reason shape `invalid metavariable name ... in: ...`, parse-time bare envelope — text carries the bare `reason:` line only, JSON carries explicit nulls), pinned text+JSON+untouched in `metavar_space_verbatim_e2e.rs`.

It gains one further member: the Section 9.4 duplicate-kind fence (reason shape `metavariable ... declared twice with different kinds (prev vs new)`, rule+file envelope with null range/suggestion), pinned text+JSON+untouched in `duplicate_kind_verbatim_e2e.rs`.

Two further structural pattern rejections are fenced below: an RSmPL
hunk with no pattern body lines (rule-only envelope), and a `#`
directive line after body lines started (reason + set suggestion
envelope, Section 7.3).

```text
RSMPL_PATTERN_CONFLICT
reason: unknown directive `#foo=bar`: # foo=bar
suggestion: check SPEC 9.3 for the supported directive list
```
The fence above is the canonical `k=v` spelling; Section 9.6 carries the other two engine-verbatim variants (the typo near-miss with its `use ...` suggestion and the bare first-token-echo form) plus the defensive-only `invalid directive:` note. Every unknown-directive shape carries a `fix_directive` repair (SPEC D.2): the near-miss names the corrected line (`# alow=many` → `# allow=many`), the far miss above repairs with `did_you_mean: null`.

Any `# mode=tokens` hunk that is not a macro token-tree pattern is
rejected (only that shape supports token matching in v0.1), as is any
hunk whose `+` side uses a metavariable bound nowhere in `-`/context
(no silent unbound invention). Engine-verbatim (both exit 2; both
carry `rule:`/`file:`; text omits null `range:`/`suggestion:` lines,
JSON carries explicit nulls):

```text
RSMPL_PATTERN_CONFLICT
rule: x
file: a.rs
reason: `# mode=tokens` is only supported for macro token-tree patterns in v0.1
```

```text
RSMPL_PATTERN_CONFLICT
rule: y
file: a.rs
reason: metavariable(s) used in `+` pattern without binding: F
suggestion: bind them in `-`/context or declare `# use=`
```

In the second fence `F` is the offending-name list (`, `-joined when
several); the text `suggestion:` line is present only there (null
suggestion omitted in the first fence, explicit `suggestion: null` in
JSON for the first, `range: null` in both).

The body list above gains one final member: a degenerate empty `-`/`+`
pattern pair is the one reachable `PatternKind::Raw` shape. Every other
classifier gate (fn, item, `use`, macro, call, generalized let/expr/type)
passes an empty pattern by, so the plan stage rejects it with
`RSMPL_PATTERN_CONFLICT` (exit 2; `rule:` + `file:` lines; text omits the
null `range:`/`suggestion:` lines, JSON carries explicit nulls):

```text
RSMPL_PATTERN_CONFLICT
rule: r
file: a.rs
reason: unsupported pattern shape in v0.1 (only fn rename, struct/enum/trait/union/type/const/static/mod/impl item rename, `use`-path prefix rename, call rewrite, let-stmt/general-expr/type rewrite, item delete/insert, macro path rewrite)
```

An RSmPL hunk whose directive block is followed by zero body lines is
the empty-body rejection (exit 2; `rule:` + `reason:` only — text omits
the null `file:`/`range:`/`suggestion:` lines, JSON carries `rule` set
with the rest null), plus a body-template `fix_pattern` repair:

```text
RSMPL_PATTERN_CONFLICT
rule: r
reason: RSmPL hunk has no pattern body lines
```

A `#` directive line after body lines started is the
misplaced-directive rejection (exit 2; reason + set suggestion — text
omits the null `rule:`/`file:`/`range:` lines, JSON carries explicit
nulls; Section 7.3 documents the same fence), plus a `fix_pattern`
repair naming the same move:

```text
RSMPL_PATTERN_CONFLICT
reason: directive line must appear immediately after hunk header, before body lines
suggestion: move the `#` line above the first `-`/`+`/` ` line
```
Directive-parse failures are not uniform (all exit 2). A bare unknown
directive (`# foo=bar`) carries no `rule:`/`file:`/`range:` lines (null
fields are omitted from text output; the JSON envelope carries explicit
`rule: null`/`file: null`/`range: null`) plus its `fix_directive`
repair, while an unknown header tag
carries `old_range: line 2` — the hunk-header line number, not a
`start..start+len` range — with null rule/file (verbatim in Section 8.3)
plus its `fix_directive` repair (`rule` itself when close, else a
removal).
Later-stage pattern failures
(item shapes, prefixes, `# root=` — verbatim in Section 13.4, `# repeat`)
include `rule:`/`file:`
with a specific reason — the body list above enumerates the categories,
and no generic `invalid item pattern` string is ever emitted. Unknown
`# root=` values carry `fix_directive` (`# root=exrp` →
`# root=expr`); every other gate in the body list carries the `fix_pattern`
template its fence names, and the engine boundary backstops any bare
conflict with a keyed concrete template (WI-2b hygiene, SPEC D.2).

### 25.4 Match conflict

No match or too many matches.

```text
RSMPL_MATCH_CONFLICT
rule: rename_calls
file: src/lib.rs
old_range: 40..48
reason: expected one match, found 2
suggestion: add `# allow=many` if multiple rewrites are intended
```

(Text mode prints `reason:`/`suggestion:` lines; the count saturates at
2 under `# allow=one`, so the example never shows an exact total — see
the saturation note below. JSON mode carries the same `reason` string in
its `{version, error: {...}}` envelope, Section D.2.) The `old_range` is
`start..start+len`, so the canonical `@@ -40,8 @@` range reports `40..48`
(Section 25.1 states the same convention).

A zero-match failure names the file to read instead:

```text
RSMPL_MATCH_CONFLICT
rule: rename_calls
file: src/lib.rs
old_range: 40..48
reason: no match found
suggestion: read `src/lib.rs` to list valid ranges; check the pattern or add `# optional`
```

The fence above is the empty-file case: with no source lines there is
nothing to preview and no elsewhere match to retry, so both lines stay
bare.

When the engine can prove the range (not the pattern) is wrong, it appends
a zero-discovery retry (T62 probe: the same matcher re-run whole-file,
count-only) to the `suggestion:` line, after ` — `; the `reason:` line
keeps only the preview form documented below. Exactly one match elsewhere
appends `— exactly one match exists elsewhere in the file; retry with
`# scope=file` + `@@ -1,1 @@``; several matches elsewhere append
`— multiple matches exist outside this range; widen the range (or add
`# scope=file`) and add `# allow=many` if intended`. (Counts saturate:
matchers stop counting at 2 under `# allow=one`, so the many-case never
claims an exact number.)

Engine-verbatim (exit 2; `suggestion:` carries the base text plus the
appended tail):

```text
RSMPL_MATCH_CONFLICT
rule: rename_calls
file: src/lib.rs
old_range: 2..3
reason: no match found — nearest source at line 2: 'let x = 1;'
suggestion: read `src/lib.rs` to list valid ranges; check the pattern or add `# optional` — exactly one match exists elsewhere in the file; retry with `# scope=file` + `@@ -1,1 @@`
```

```text
RSMPL_MATCH_CONFLICT
rule: rename_calls
file: src/lib.rs
old_range: 2..3
reason: no match found — nearest source at line 2: 'let x = 1;'
suggestion: read `src/lib.rs` to list valid ranges; check the pattern or add `# optional` — multiple matches exist outside this range; widen the range (or add `# scope=file`) and add `# allow=many` if intended
```

(JSON mode carries the same `reason`/`suggestion` strings in its
`{version, error: {...}}` envelope with `rule:`/`file:` set and `range:`
echoing the hunk range, Section D.2.)

The zero-match `reason` also carries a T67 candidate preview, appended
after ` — `: the most similar same-kind item header (`nearest fn foo at
line 5: 'fn foo() {}'`), or the nearest non-empty source line
(`nearest source at line 1: 'fn f() {}'`) when the hunk kind has no item
headers — so the next attempt often needs no file read. The header walk reaches block-level items
(fn bodies, closures, method bodies — the same scope matching reaches), and
same-kind headers win over a nearer enclosing item (a typo'd `struct Lcoal`
previews the real `struct Local`, not the enclosing `fn`). Within the
same-kind set, name similarity (edit distance) ranks before line distance:
a typo'd `struct Betta` previews the real `struct Beta` one line further
down, not the nearer `struct Alpha`. The preview is a single line of at most
200 chars of source (plus a short `kind name at line N` label, whole string
bounded); `suggestion` is unchanged. An empty file has nothing to preview
and keeps the bare `no match found` reason. If the preview shows a

Engine-verbatim (exit 2):

```text
RSMPL_MATCH_CONFLICT
rule: rename_fn
file: src/lib.rs
old_range: 1..3
reason: no match found — nearest fn bar at line 2: 'fn bar() {}'
suggestion: read `src/lib.rs` to list valid ranges; check the pattern or add `# optional`
```

```text
RSMPL_MATCH_CONFLICT
rule: rename_calls
file: src/lib.rs
old_range: 1..2
reason: no match found — nearest source at line 1: 'fn f() {}'
suggestion: read `src/lib.rs` to list valid ranges; check the pattern or add `# optional`
```

(JSON mode carries the same strings with `rule:`/`file:` set and `range:`
echoing the hunk range; nulls only where a line is absent — never here.)
`let-else` target with a `let` pattern that carries no diverge,
a `match` / loop / closure expression,
a `P @` shape with a concrete head or composite subpattern, or a macro type, the shape itself is outside v0.1
scope (Section 18.6 rows R1 / R2 / R4 / R5) — rewrite it with an
ordinary hunk instead of re-shaping the pattern.
The same holds when the preview shows a `use` nested inside a function body:
only file- and inline-`mod`-level `use` items match (Section 14.9), so a
fn-body `use` is ordinary-hunk territory too.

A matcher-stage partial overlap is a different `RSMPL_MATCH_CONFLICT` shape from the zero/too-many fences above (which carry `old_range:` plus a set suggestion): two accepted candidates that partially overlap without nesting fail with `reason: partially overlapping matches cannot be merged` (`rule:` + `file:` lines; text omits the null `range:`/`suggestion:` lines, JSON carries explicit `range: null` / `suggestion: null`; suggestion is null) plus an `overlap`-template `fix_pattern` repair (narrow the range or split the hunks). The canonical fence lives in Section 17.6, which also contrasts it with the plan-stage `overlapping edits cannot be merged` (`GIT_HUNK_CONFLICT`, Section 25.7).

The `# repeat` nesting backstop shares the rule+file shape (but sets
`suggestion:`, unlike the suggestion-null partial-overlap fence above):
`reason: repeat nesting exceeds 64 levels (pathological nesting?)` with
`suggestion: remove `# repeat` or simplify the nesting` — canonical
fence in Section 17.8 — plus a `repeat`-template `fix_pattern` repair
naming the same exit.

### 25.5 Symbol conflict

Symbol resolution failed. A bare `# symbol` hunk verifies its
minus-side seeds against the crate-local index (Section 19.3); a seed
that resolves to no definition fails with `RSMPL_SYMBOL_CONFLICT`:

```text
RSMPL_SYMBOL_CONFLICT
rule: rename_symbol_calls
file: src/lib.rs
reason: symbol `Old` (`crate::old_api`) resolves to no definition in this crate
suggestion: write the concrete path `crate::old_api` in the pattern, or add `# fallback=syntax` to allow syntactic matching
```
 
`Old` is the declared `# symbol Old=...` metavariable name (backticked in the reason). Envelope (exit 2): text carries `rule:`/`file:` lines plus the reason and a set `suggestion:` line (the null `range:` is omitted — no `old_range:` line); the JSON envelope carries `rule`/`file` set, `range: null`, and the same set suggestion. Suggestion is set in both envelopes here, unlike the suggestion-null C14/C15 rows (the `use a call-shape hunk ...` tail there is part of `reason:`).

### 25.6 Capture conflict

A capture is missing or invalid.

```text
RSMPL_CAPTURE_CONFLICT
rule: paste_helper
file: src/a.rs
reason: capture `Helper` is not bound by any hunk
suggestion: add a hunk with `# bind=<Name>`
```

When other captures are bound elsewhere in the patch, the suggestion
names them instead (sorted, `", "`-joined):

```text
RSMPL_CAPTURE_CONFLICT
rule: paste_missing
file: src/a.rs
reason: capture `Missing` is not bound by any hunk
suggestion: available captures: Known
```

Both fences share one envelope (exit 2; `rule:` + `file:` lines; text
omits the null `range:` line, JSON carries explicit `range: null`
with the same `reason`/`suggestion` strings; suggestion is set in
both — the empty-known branch points at the `# bind=<Name>` syntax,
the non-empty branch lists the bound names, e.g.
`available captures: Alpha, Zulu`). In the reason `{name}` echoes the
offending `# use=` name; in the first suggestion `<Name>` is a literal
placeholder, not the missing name.

Captures are patch-local, not document-ordered: a `# use=Helper` paste
may precede its `# bind=Helper` cut (forward references, Section 20.3).
A second capture failure needs no missing name: a hunk declaring both
`# bind=<Name>` and `# allow=many` on a non-item shape fails with `RSMPL_CAPTURE_CONFLICT`
(reason: `` v0.1 `# bind=` is only supported on item-capture cut hunks (`- I`) ``
— the same envelope as the Section 20.3 kind fence). On an item cut the
combination is the ordered collection (Section 20.3), never a failure. The failure carries `rule:`/`file:`
like the shape fences in Section 20.3 (text omits the null `range:`/`suggestion:` lines,
JSON carries explicit nulls); suggestion is null — the two missing-capture
fences above are the only 25.6 entries with a set suggestion.
A third needs no separate hunk either: one hunk carrying both `# bind=H` and `# use=H` fails with the Section 20.3 self-cycle fence (same `rule:`/`file:` envelope, null suggestion).

### 25.7 Git hunk conflict

A normal Git hunk failed.

```text
GIT_HUNK_CONFLICT
rule:
file: src/lib.rs
old_range: 1..5
reason: removal mismatch at line 2: expected `wrong`
```

The `rule:` line is empty (ordinary hunks carry no rule name; JSON reports
`"rule": ""`), and `old_range` follows the same `start..start+len` convention
(`@@ -1,4 @@` → `1..5`). The `reason` names the failing line and the
expected text — one of four engine-verbatim shapes (all exit 2; all share
the empty-`rule:` / present-`old_range:` / null-suggestion envelope above).
The two mismatch shapes echo the failing line and its expected text; the
two EOF shapes fire when the body walks past the end of the file. Note the
asymmetry: the context arm says `context line beyond EOF`, the removal arm
says `removal beyond EOF` (no `line` word):

```text
GIT_HUNK_CONFLICT
rule:
file: src/lib.rs
old_range: 1..2
reason: context mismatch at line 1: expected `WRONG`
```

```text
GIT_HUNK_CONFLICT
rule:
file: src/lib.rs
old_range: 1..2
reason: context line beyond EOF (line 2)
```

```text
GIT_HUNK_CONFLICT
rule:
file: src/lib.rs
old_range: 1..2
reason: removal beyond EOF (line 2)
```

The two mismatch shapes also carry machine repair (text `repair:` line +
JSON `repair` object): `{"action":"fix_context","line":N,"expected":"...",
"actual":"..."}` — `N` is the 1-based original-file line, `expected` the
hunk's line text, `actual` the file's line text (full single-line texts,
never truncated). The two EOF shapes carry `fix_range` instead (SPEC
D.2): there is no actual line to show, but the clamped header tells the
agent where the file ends — paste it and retry.
Both mismatch shapes also carry a content-discipline `suggestion:` (the file line differs from the hunk text: paste `actual` from the repair or re-read the file) — by the time the engine runs, the parser has already stripped exactly one marker char and proved header counts equal body counts, so a missing marker cannot cause this error. Blank-involved mismatches carry the empty-context sentence instead: blank file lines are empty context, written as ` ` (single space) or an empty line inside the hunk body.

Three further engine arms never surface via the CLI, so they carry no fence
here (defensive backstops for library callers, documented but not pinnable):
`bad range` (a `LineIndex` miss after bounds checks passed), an `invalid diff
prefix` rejection (the patch parser ends the hunk at any non-body line, so
the engine never sees one — a `?` line yields an empty body and an empty
lowering), and `target file not found in tree: {path}` (the CLI pre-fills
every missing target as empty plus a `treating as new (empty) file` warning,
so the plan-stage lookup never misses; Section 27.6).

An overlap conflict is a different `GIT_HUNK_CONFLICT` shape from the content
failure above: it fires when two edits overlap in original coordinates
(semantic-semantic, ordinary-semantic, or an insertion strictly inside a
deleted/replaced range), regardless of which hunk kinds produced them.
Engine-verbatim (exit 2):

```text
GIT_HUNK_CONFLICT
file: src/lib.rs
reason: overlapping edits cannot be merged
suggestion: apply the overlapping rewrites in separate runs, or make the ranges disjoint
```

Text carries `file:` but no `rule:`/`old_range:` lines (nulls omitted; JSON
carries explicit `rule: null` / `range: null` with `file` set and the same
`reason`/`suggestion` strings). Contrast:

| conflict | `rule` | `old_range`/`range` | `suggestion` |
|---|---|---|---|
| ordinary-hunk content mismatch | `""` (empty line in text) | present (`start..start+len` / JSON `range`) | present (content discipline; the blank arm carries the empty-context sentence) |
| overlap (`overlapping edits ...`) | absent in text / `null` in JSON | absent in text / `null` in JSON | present (repair pointer) |

The overlap shape carries a `fix_pattern` repair beside the suggestion
(`apply the overlapping rewrites in separate runs, or make the ranges
disjoint` — applicable by splitting the patch, retrying per piece).

Disjoint edits in one patch merge normally (Section 10.6 option 1).

A git-spawn failure is a third `GIT_HUNK_CONFLICT` shape: the lowered
diff was computed, but the `git apply` / `git apply --check` child
could not be spawned (`git` missing from `PATH`; `PATH` lookup, no
`--git-path` override). Engine-verbatim (exit 2, apply mode — text
stdout is empty; `--check` text mode prints the lowering on stdout
and this envelope on stderr, already pinned by
`failing_check_still_writes_output`):

```text
GIT_HUNK_CONFLICT
reason: failed to run `git apply` (is git installed?): No such file or directory (os error 2)
```

The trailing `<os error>` is platform-dependent (Linux reports
`No such file or directory (os error 2)`); pins assert the verbatim
`` failed to run `git apply` (is git installed?):  `` prefix. Text
carries the `reason:` line plus the `repair:` line (no `rule:`/`file:`/
`old_range:`/`suggestion:` lines; JSON carries explicit `rule: null` /
`file: null` / `range: null` / `suggestion: null` with the same
`reason` string and a non-null `repair`). Unlike the overlap shape above,
the spawn shape carries no `file:`. The repair is `fix_pattern` with an
environment template for spawn/send/wait faults (`ensure git is
installed ...`) and a stale-range template for a rejected lowering
(`re-read the target files and retry ...`; a persistent failure there
is an engine lowering bug, not a patch bug). Full fences live in
Section 31.7; the two sibling
send/wait arms (`failed to feed patch to git: <os error>`,
`` failed waiting for `git apply`: <os error> ``) share this
context-less envelope by construction but never surface via the CLI
(defensive backstops, documented in 31.7, not pinnable).

### 25.8 I/O conflict

A filesystem operation failed: the patch file or stdin could not be read,
a repo file could not be read, or the `--output` file could not be written.
Filesystem failures use this code so JSON-mode agents always receive a
machine-readable envelope. Flag-usage errors (unknown flag, bad value)
use this code too, for the same reason:
with `--format json` they print the stdout-only envelope (D.2) instead of
clap's stderr text.

```text
RSMPL_IO_CONFLICT
file: change.rsmpl
reason: cannot read patch file change.rsmpl: No such file or directory (os error 2)
suggestion: check that `change.rsmpl` exists and is readable, then retry
```

 Null fields are omitted from text output (no `rule:`/`range:` lines above); the JSON envelope carries explicit `rule: null` / `range: null`. The unreadable patch file is the WI-3 allow-list entry: the failure stays repair-null (environmental — no patch spelling fixes it) while the `suggestion` names the path instead.

A repo-file read failure reports `reason: cannot read <abs repo-joined>: <os error>` with `file:`
echoing the repo-relative target (exit 2; text omits null `rule:`/`range:`/`suggestion:` lines, JSON
explicit nulls; the `<os error>` tail is platform-dependent; fires while loading patch targets, before any
matching or lowering, so the worktree is untouched in both `--check` and apply modes). Contrast the
patch-file twin above: that reason carries the `patch file` infix and echoes the `--file` argument in
both `file:` and the reason, while the repo twin carries no `patch file` infix and echoes the absolute
repo-joined path in the reason. The `cannot read stdin: <os error>` and
`cannot serialize JSON report: <os error>` arms are defensive-only: stdin IO errors are not triggerable via
the CLI (the shell owns fd redirection) and the JSON payload is plain strings, so neither is pinnable --
documented, not pinned (T287/T290/T291 precedent).

An unwritable `--output` target reports `reason: cannot write <path>: <os error>` with `file:` echoing the `--output` path, and aborts before checking or applying (`--output` is written before `--check` runs, Section D.2.3). Its empty-lowering twin — a pre-existing file the mirror removal cannot delete — reports `reason: cannot remove stale <path> (empty lowering): <os error>` with the same envelope (exit 2; `file:` set to the `--output` path; text omits null `rule:`/`range:`/`suggestion:` lines, JSON explicit nulls; full fence in Section D.2.3).

Patch `target_path` values that are absolute or escape the worktree
root fail here too (Section 27.6 lists both engine-verbatim reasons) — before any file is read. The code stays `RSMPL_IO_CONFLICT` (T94 pin) but the failure carries a `paths`-template `fix_pattern` repair (use a repo-relative `diff --git` target). Differing
`diff --git a/old b/new` paths (a file rename, Section 6) fail as a
parse conflict (25.2), not an I/O conflict: the paths are well-formed,
the operation is unsupported.

### 25.9 Repair completeness (WI-3)

Every error class reachable from patch input carries a machine `repair`
(SPEC D.2 shapes); the `wi3_repair_complete_e2e.rs` enumeration pins one
failing patch per row and fails CI if any producer loses its repair.
Paste `write_instead` verbatim (or follow the named next action) and
retry — never parse the English `reason` to recover.

| code | class | repair action | notes |
|---|---|---|---|
| `RSMPL_RANGE_CONFLICT` | range past EOF | `fix_range` | largest in-file range; insertion anchors clamp to EOF (25.1) |
| `RSMPL_RANGE_CONFLICT` | span guards / internal edit out of bounds | `fix_pattern` (`internal`) | defensive-only; persistent failure is an engine bug (25.7) |
| `RSMPL_PARSE_CONFLICT` | malformed header / range / elided ordinary range | `fix_pattern` (`header`) / `fix_header` | drop-in header (25.2) |
| `RSMPL_PARSE_CONFLICT` | outside-section line / empty patch / no body / misplaced directive / bad quote / bad name | `fix_pattern` (section/body/directive/quote/name template) | concrete spelling to write (25.2/25.3) |
| `RSMPL_PARSE_CONFLICT` | target file missing / unparsable / rewritten file unparsable | `fix_pattern` (`paths` / `syntax`) | fix paths, or fix the syntax first (ordinary hunks bypass the parse) |
| `RSMPL_PATTERN_CONFLICT` | unknown directive / header tag / `# root=` | `fix_directive` | gated suggestions, `null` when far (25.3, D.2) |
| `RSMPL_PATTERN_CONFLICT` | invalid directive value | `fix_directive` | corrected line, or the full legal set when far (25.3, D.2) |
| `RSMPL_PATTERN_CONFLICT` | all other compile gates | `fix_pattern` (C-code or gate-name template) | engine boundary backstops bare conflicts as `pattern` (D.2) |
| `RSMPL_MATCH_CONFLICT` | zero / too many / postcondition | `widen_scope` / `read_file` / `add_directive` | (25.4, D.2) |
| `RSMPL_MATCH_CONFLICT` | partial overlap | `fix_pattern` (`overlap`) | defensive backstop — narrow the range or split the hunks (25.4) |
| `RSMPL_MATCH_CONFLICT` | attribute policy mismatch at match time | `fix_pattern` (`attrs`) / `add_directive` (`# attrs=preserve`) | drop the `attrs` directive to the ignoring default, or write the attributes into the pattern |
| `RSMPL_MATCH_CONFLICT` | repeat-depth backstop / wrong field-type annotation | `fix_pattern` (`repeat` / `type`) | remove `# repeat`, or write the field's real type (25.4) |
| `RSMPL_MATCH_CONFLICT` | name-span guards | `fix_pattern` (`internal`) | defensive-only; persistent failure is an engine bug |
| `RSMPL_SYMBOL_CONFLICT` | unresolved / vetoed follow | `add_directive` (`# fallback=syntax`) | (25.5, D.2) |
| `RSMPL_CAPTURE_CONFLICT` | unbound / duplicate capture | `fix_name` | rename or add `# bind=` (25.6, D.2) |
| `RSMPL_CAPTURE_CONFLICT` | self-cycle / kind-misuse | `fix_pattern` | split the hunks (25.6) |
| `GIT_HUNK_CONFLICT` | ordinary mismatch / EOF | `fix_context` / `fix_range` | paste actual, or paste clamped header (25.7) |
| `GIT_HUNK_CONFLICT` | overlap | `fix_pattern` (`overlap`) | split into separate runs (25.7) |
| `GIT_HUNK_CONFLICT` | git spawn / feed / wait / rejected lowering | `fix_pattern` (`environment` / `stale`) | install git, or re-read and retry (25.7, 31.7) |
| `RSMPL_IO_CONFLICT` | unreadable patch file | none (`null`, allow-list) | environmental; `suggestion` names the path (25.8) |
| `RSMPL_IO_CONFLICT` | unreadable repo/output file | none (`null`) | environmental; `file` + `reason` name the path (25.8) |
| `RSMPL_IO_CONFLICT` | absolute / escaping target | `fix_pattern` (`paths`) | code stays IO (T94 pin); fix the target spelling (25.8) |
| `RSMPL_IO_CONFLICT` | bad flags | none (`null`) | not patch-input reachable; suggestion names the fix (D.2) |
| `RSMPL_BUILD_CONFLICT` | post-lowering `cargo check` failure | `read_file` | diagnostic location wins, else first orphan; null only on the location-less double-miss (allow-list, 25.10) |
| `RSMPL_IO_CONFLICT` | missing `Cargo.toml` / missing `cargo` under `--verify build` | none (`null`, allow-list) | environmental; `suggestion` names the fix (25.10) |

The IO rows, the build-environmental row, and the build double-miss
row above are the only classes that may carry a null `repair`:
environment failures have no patch spelling to correct (partial
overlap is a defensive backstop, but even it carries the `overlap`
next action). Everything else is machine-applicable by
construction.

WI-18 source gate: `rsmpl_patch::errors` tests enumerate every
`StructuredError::new` site in production code (`crates/*/src`,
`bin/*/src`). Non-filler codes (`Match`/`Range`/`Parse`/`GitHunk`/
`Build`) must carry a site-level `with_repair` — the filler does not
cover those codes, so a bare site there is a prose-only failure.
`Pattern`/`Symbol`/`Capture` bare sites are the engine-tail class
(`fill_generic_repair` at the `apply_inner` boundary keys one
concrete template per class and never echoes the English reason);
their count is ceiling-pinned so the class never grows without an
explicit site-level repair. `IoConflict` bare sites are the
environmental allow-list. The runtime companion is
`wi3_repair_complete_e2e.rs` (one failing patch per patch-input
class).

---

### 25.10 Build conflict

A post-lowering `cargo check` failure under `--verify build`: the
lowering applied cleanly but the resulting tree does not compile. Both
modes fail loud with exit 2 — `--check` leaves the worktree untouched
(`applied: false`, verified in a temp copy) while apply mode keeps the
lowering (`applied: true`, carried as the top-level JSON `applied`
key) — so the agent always knows the tree state.

```text
RSMPL_BUILD_CONFLICT
file: src/lib.rs
old_range: 2..2
reason: --verify build: error[E0425]: cannot find function `undefined_fn` in this scope (patch stays applied)
suggestion: read `src/lib.rs` at `2..2` and fix the build breakage, then retry
repair: {"action":"read_file","file":"src/lib.rs","range":"2..2"}
```

`file`/`range` name the first cargo ` --> file:line` diagnostic
(`range` is the `L..L` single-line range); when no diagnostic location
parses, the first orphan's `file`/`line` is used instead (the report
already lists it). Only a location-less failure with no orphans stays
repair-null (allow-listed, 25.9). `rule` is null (the failure is not
tied to one hunk); text mode omits the null `rule:` line, JSON carries
explicit nulls.

Environmental failures keep `RSMPL_IO_CONFLICT` (25.8) with null repair:
a missing `cargo` binary or a missing `Cargo.toml` (`could not find
Cargo.toml`). The `reason` names the cause; the `suggestion` names the
fix.

## 26. Lowering to a Concrete Git Diff

An RSmPL-Git engine should support lowering.

### 26.1 Lowering pipeline

```text
RSmPL-Git patch
    -> parse ordinary and semantic hunks
    -> apply semantic hunks to worktree or virtual tree
    -> apply ordinary hunks
    -> produce modified tree
    -> run git diff
    -> output concrete Git patch
```

### 26.2 Review output

The concrete diff should contain no RSmPL directives.

Input:

```diff
@@ -10,7 @@ rule=rename_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }
```

Output after lowering might be:

```diff
@@ -10,3 +10,3 @@
-fn foo(x: i32) -> i32 {
+fn bar(x: i32) -> i32 {
     x + 1
 }
```

The lowered hunks carry a unified-diff context radius of 0: only changed
lines are shown around each change (T128 fixed lowering — no radius lever).
This spends the fewest review tokens; per-file added/removed line counts
are computed from the full old/new contents and never change with the report
budget. A zero-context lowering is still a valid unified diff, but plain
`git apply` rejects zero-context hunks by default as a safety measure — it
needs `git apply --unidiff-zero` (which the reference CLI passes
automatically for every check/apply). Lowering stays deterministic given the
same inputs (see D.2.2).

### 26.3 Determinism

Given the same input tree, patch, formatter settings, and semantic environment, lowering should be deterministic.

---

## 27. Security and Safety Considerations

### 27.1 No hidden global search

RSmPL must not rewrite outside declared hunk ranges.

This is critical for LLM-generated patches.

### 27.2 No silent semantic fallback

If semantic resolution is requested and unavailable, the engine should fail unless the patch explicitly allows fallback.

### 27.3 Macro expansion caution

Macro-expanded rewriting can be dangerous because source spans may be ambiguous. It should be opt-in.

### 27.4 Build script caution

A full Rust semantic environment may require running build scripts. Implementations should provide a safe mode that avoids executing untrusted build scripts.

### 27.5 Reviewability

All semantic changes should be lowerable to concrete Git diffs before commit.

### 27.6 Path containment and new-file creation

Patch `target_path` values must be repo-relative: no leading `/`
(which would discard `--repo` under path joining) and no `..`
segments that escape the worktree root. The check is lexical
(`.`/empty segments ignored, symlinks unresolved); violations fail
with `RSMPL_IO_CONFLICT`. `--repo` itself is a trusted invoker
argument, not patch content.
  The engine reports these path reasons verbatim (`file:` echoes the target path; both carry no suggestion — null in JSON, line omitted in text):
  ```text
  target path is absolute (/tmp/evil.rs); use a repo-relative path without leading /
  target path escapes the repo (../evil.rs); use a repo-relative path without ../
  ```
A missing target file warns
(`treating as new (empty) file`) and is treated as empty, so a
typo'd path creates a stray file on apply — read the warning before
committing; `--format json` consumers must also read stderr, where
all `warning:` lines live (see D.2.3). Created files lower with
`new file mode 100644` + `--- /dev/null` headers so the concrete
diff applies with a bare `git apply`; deleted files mirror it with
`deleted file mode 100644` + `+++ /dev/null` headers (T350) —
`120000` when the target is a symlink, whose content reads as its
target string, never followed (T353). Creates honor the input
section's declared `new file mode` (`100644` default, `120000` for
symlink creates); ordinary hunks preserve a missing trailing
newline instead of appending one (T355). A same-path delete+create
pair (git's file<->symlink typechange shape) lowers as the pair
when the declared modes differ, else a single plain modify (T357).
Empty sides lower as headers only (exactly what real git emits for
empty files, no hunks): an empty create is created, an empty delete
is removed (T364 -- the old `None` silently dropped both). A pure
mode change (content equal, `old mode`/`new mode` differ) lowers as
a headers-only flip section; mode lines also ride along content
changes, spliced after the `diff --git` header (the splice keeps body
bytes exact -- CRLF `\r` survives, T402). A declared `100755`
(untracked executables, unix) is honored like `120000`.
Emitted `diff --git`/`---`/`+++`/`rename from/to` slots C-quote paths
holding control characters, `"` or `\` (bare git cannot parse them
raw — it never emits them raw either); spaces and non-ASCII stay raw,
the native shape (T398).
`--output <PATH>` is resolved
against the caller cwd, never joined to `--repo`: it is the agent's
local artifact, not repo content. File renames are not a containment
issue: differing `diff --git` paths are rejected earlier as a parse
conflict (Sections 6, 25.2).

---

## 28. Recommended LLM Emission Style

Fences tagged `diff rsmpl` below are executable: `spec_examples`
extracts and checks every one (Section 29 intro).

### 28.1 Use ordinary Git hunks for simple line edits

Good:

```diff
@@ -1,3 +1,4 @@
 mod a;
+mod b;
 fn main() {}
 }
```

Do not use RSmPL for trivial additions unless semantic placement is important.

### 28.2 Use RSmPL for semantic renames

Good (diet form: elided new side, inferred `E`, no `# lang=rust`
next to other directives — Sections 7.1, 10.4, 15.8):

```diff rsmpl
@@ -40,8 @@ rule=rename_calls
# allow=many
- old_api(E)
+ new_api(E)
```

### 28.3 Use ranges aggressively

Keep RSmPL ranges tight: cite the smallest range that contains the match
(read the file for exact spans). When the agent already holds
a small file, skip range discovery entirely with whole-file scope:

```diff rsmpl
diff --git src/lib.rs
@@ -1,1 @@ rule=rename_one_block
# sweep
- old_api(E)
+ new_api(E)
```

The `@@ -1,1 @@` range is a dummy (SPEC 17.3) — no line counting, no
discovery roundtrip.

Better (tight range):

```diff
@@ -80,15 @@ rule=rename_one_block
- old_api(E)
+ new_api(E)
```

Worse (whole-file range: forces the author to count the file length, and
the range goes stale on every edit):

```diff
@@ -1,500 @@ rule=rename_whole_file
- old_api(E)
+ new_api(E)
```

### 28.4 Declare multiple matches explicitly

If the patch expects many call sites:

```text
# allow=many
```

### 28.5 Do not abstract too much

Prefer concrete patterns where possible.

Better:

```diff
- client.send(E)
+ client.try_send(E)?
```

Worse:

```diff
- P(E)
+ Q(E)
```

unless path abstraction is needed.

### 28.6 Use semantic symbols for real renames

For cross-scope rename safety (copy-pasteable; v0.1 matches the full path
exactly, so prefer concrete paths — Section 19.3):

```diff rsmpl
diff --git src/lib.rs
@@ -5,2 @@ rule=rename_symbol
# allow=many
# fallback=syntax
# symbol Old=crate::old_api
# symbol New=crate::new_api
- Old(...)
+ New(...)
```

### 28.7 Use cut/paste for moves

Do not restate large moved code blocks.

Use:

```diff
# bind=Moved
- I
```

and later:

```diff
# use=Moved
+ Moved
```

(Exposition fragments — the complete executable move is §29.6.)

---

## 29. Examples

All examples use the diet-minimal recipe from Section 6 (single-path
header, elided new side, inferred metavariables with no `# expr`
declaration, no `# lang=rust` next to other directives — the form
agents should emit). Bare renames use the terse form plus `rule=`
with no directive lines (Section 7.1). Fences tagged `diff rsmpl`
are executable: `bin/applyrs/tests/spec_examples.rs` extracts and
checks every one (hunk-only fences without a `diff --git` line are
completed with `diff --git src/lib.rs` before checking).

### 29.1 Rename a plain function

```diff rsmpl
diff --git src/lib.rs
@@ -10,7 @@ rule=rename_plain_foo
- fn foo
+ fn bar
```

Matches:

```rust
fn foo() {}
```

Matches:

```rust
fn foo(x: i32) -> i32 {
    x + 1
}
```

Does not match:

```rust
pub async unsafe fn foo<T>(x: T)
where
    T: Clone,
{
    x
}
```

### 29.2 Rename a public async unsafe generic function

```diff rsmpl
diff --git src/lib.rs
@@ -30,12 @@ rule=rename_complex_foo
- pub async unsafe fn foo<T>(...) where T: Clone {
+ pub async unsafe fn bar<T>(...) where T: Clone {
      ...
  }
```

This only matches functions with the explicitly written shape.

### 29.3 Rename calls in a known range

```diff rsmpl
diff --git src/lib.rs
@@ -40,8 @@ rule=rename_calls
# allow=many
- foo(E)
+ bar(E)
```

### 29.4 Method call to free function

```diff rsmpl
diff --git src/service.rs
@@ -75,20 @@ rule=method_to_free_function
# allow=many
- Receiver.old_method(Arg)
+ new_function(Receiver, Arg)
```

### 29.5 Semantic rename with symbol resolution

```diff rsmpl
diff --git src/lib.rs
@@ -80,20 @@ rule=rename_symbol_calls
# allow=many
# fallback=syntax
# symbol Old=crate::old_api
# symbol New=crate::new_api
- Old(...)
+ New(...)
```

### 29.6 Move a function between files

```diff rsmpl
diff --git src/old.rs
@@ -20,12 @@ rule=cut_helper
# bind=Helper
- I

diff --git src/new.rs
@@ -5,0 @@ rule=paste_helper
# use=Helper
+ Helper
```

### 29.7 Mixed RSmPL and ordinary Git patch

```diff rsmpl
diff --git src/lib.rs
@@ -10,7 @@ rule=rename_foo
- fn foo(...) {
+ fn bar(...) {
      ...
  }

@@ -40,8 @@ rule=rename_calls
# allow=many
- foo(E)
+ bar(E)

@@ -1,3 +1,4 @@
 mod old;
+mod new;

 fn main() {}
```

### 29.8 Insert a use item semantically

```diff rsmpl
diff --git src/lib.rs
@@ -3,0 @@ rule=insert_use
+ use crate::new_module::Thing;
```

### 29.9 Rewrite inside an impl block

```diff rsmpl
diff --git src/service.rs
@@ -20,40 @@ rule=rename_impl_method
  impl Service {
      ...
-     fn old_name(...) {
+     fn new_name(...) {
          ...
      }
  }
```

The same shape with a `trait` header rewrites trait methods, including
`;` declarations (the `;` is preserved like a body). A trait-header
rename may carry `# follow=uses`: the engine then owns the
trait-proven `impl <Trait> for <Self>` definitions (WI-3), so `diffrs`
emits the single trait hunk alone:

```diff rsmpl
diff --git src/service.rs
@@ -1,3 @@ rule=rename_trait_method
  trait Service {
      ...
-     fn old_name();
+     fn new_name();
      ...
  }
```

### 29.10 Optional migration

```diff rsmpl
diff --git src/lib.rs
@@ -100,30 @@ rule=optional_old_api_cleanup
# optional
# allow=many
- old_api(E)
+ new_api(E)
```

If no calls exist in the range, the hunk succeeds without changes.

### 29.11 Stable Line Ranges Independent of Prior Hunks

This example demonstrates the original-source coordinate system from Section 10.2. The second semantic hunk keeps using the line number it had in the original file, even though the first hunk inserts a larger replacement before it.

```diff rsmpl
diff --git src/lib.rs
@@ -10,1 @@ rule=insert_large_block
- old_function()
+ new_function_that_spans(
+     many,
+     many,
+     lines,
+ )

@@ -20,5 @@ rule=rename_second_function
- fn second_function(...) {
+ fn renamed_function(...) {
      ...
  }
```

The `rename_second_function` hunk is still targeted at original line 20, exactly where `second_function` appeared in the pre-application source. The engine must not adjust that range to account for the earlier insertion. In other words, the first hunk's materialized output may shift the final file layout, but it does not rebase the old-side range of later RSmPL hunks.

---

## 30. Formal Grammar Sketch

This grammar is illustrative, not a complete parser specification.

```ebnf
patch               ::= file_section+

file_section        ::= git_file_header file_metadata* hunk*

git_file_header     ::= "diff --git " path (" " path)? newline

file_metadata       ::= metadata_line newline

hunk                ::= ordinary_hunk | rsmpl_hunk

ordinary_hunk       ::= hunk_header ordinary_hunk_body

rsmpl_hunk          ::= hunk_header directive_block rsmpl_hunk_body

hunk_header         ::= "@@ -" range (" +" range)? " @@" hunk_header_tail? newline

range               ::= integer ("," integer)?

hunk_header_tail    ::= text

directive_block     ::= directive_line+

directive_line      ::= "# " directive newline

directive           ::= key_value_directive
                      | flag_directive
                      | metavariable_directive

key_value_directive ::= identifier "=" value

flag_directive      ::= identifier

metavariable_directive ::= metavariable_kind identifier constraint?

constraint          ::= "=" value

metavariable_kind   ::= "symbol"
                      | "expr"
                      | "stmt"
                      | "item"
                      | "type"
                      | "pat"
                      | "generic_params"
                      | "where_clause"
                      | "visibility"
                      | "qualifier"
                      | "qualifier_list"
                      | "block"
                      | "macro"
                      | "token_tree"

rsmpl_hunk_body     ::= rsmpl_body_line+

rsmpl_body_line     ::= context_pattern_line
                      | remove_pattern_line
                      | add_pattern_line
                      | no_newline_marker

context_pattern_line ::= " " rust_pattern_text newline

remove_pattern_line  ::= "-" rust_pattern_text newline

add_pattern_line     ::= "+" rust_pattern_text newline

no_newline_marker    ::= "\\" text newline
```

The `rust_pattern_text` nonterminal is intentionally summarized above. At the lexical layer, RSmPL pattern text additionally recognizes literal-token quoting before Rust parsing:

```ebnf
literal_token_quote ::= "`" rust_lexical_token "`"
rust_lexical_token   ::= identifier
                       | raw_identifier
                       | lifetime
                       | label
                       | path_segment
raw_identifier       ::= "r#" identifier_body
```

A `literal_token_quote` quotes the entire `rust_lexical_token`. For raw identifiers, this means `` `r#type` `` is valid and `` r#`type` `` is invalid.

---

## 31. Implementation Notes

### 31.1 Parser strategy

An implementation can parse in layers:

1. Parse Git file sections.
2. Parse hunk headers.
3. Detect directive blocks.
4. Route ordinary hunks to a normal patch applier.
5. Route RSmPL hunks to a Rust semantic engine.

### 31.2 Rust parser choices

Possible Rust syntax backends include:

- rustc analyzer syntax data;
- `syn`;
- tree-sitter-rust;
- rustc parser internals.

For semantic symbol resolution, a rustc analyzer backend is recommended.

### 31.3 Range mapping

The engine needs reliable byte-span to line-range mapping.

Span columns (proc-macro2 fallback) count Unicode scalars, not bytes:
the engine must convert char-column spans to byte offsets before
splicing (a multibyte char before a span end otherwise shifts the cut
and corrupts the rewritten file).

Recommended internal representation:

```text
LineRange {
    start_line: u32,
    end_line_inclusive: u32
}

ByteRange {
    start: u32,
    end: u32
}
```

The engine should translate hunk line ranges into byte ranges before AST filtering.

### 31.4 Pattern compilation

RSmPL pattern bodies should be compiled into AST-pattern objects.

Example:

```diff
# expr E
- old_api(E)
+ new_api(E)
```

could compile into:

```text
Replace {
    match_expr: Call {
        callee: Path("old_api"),
        args: [MetaExpr("E")]
    },
    replace_expr: Call {
        callee: Path("new_api"),
        args: [MetaExpr("E")]
    }
}
```

### 31.5 Exactness and whitespace

Semantic matching should ignore insignificant whitespace.

Formatting preservation should be handled during rewrite printing, not matching.

### 31.6 Comments

Comments are trivia. By default:

- comments inside matched nodes should be preserved;
- comments attached to deleted nodes should be deleted;
- comments attached to moved nodes should move with the node;
- comments outside matched nodes should remain untouched.

Implementations should document their exact comment attachment algorithm.

### 31.7 External `git` dependency

The reference CLI shells out to bare `git apply` / `git apply --check`
(`PATH` lookup, no `--git-path` override) for its final check/apply step, so
`git` must be on `PATH`. Matching, lowering, and `--output` are
pure and never spawn git.

When the child cannot be spawned (`git` missing from `PATH`), the
step fails with `GIT_HUNK_CONFLICT` (exit 2). Engine-verbatim text
envelope (apply mode; `--check` text mode prints the lowering on
stdout and this envelope on stderr):

```text
GIT_HUNK_CONFLICT
reason: failed to run `git apply` (is git installed?): No such file or directory (os error 2)
```

The trailing `<os error>` is platform-dependent (Linux reports
`No such file or directory (os error 2)`); the verbatim prefix is
`` failed to run `git apply` (is git installed?):  ``. Text carries
only the bare `reason:` line (no `rule:`/`file:`/`range:`/
`suggestion:` lines); the JSON envelope carries the same `reason`
with explicit `rule: null` / `file: null` / `range: null` /
`suggestion: null` (stdout-only, history line on stderr, SPEC D.2).

Two sibling arms share this context-less envelope by construction
(`git_error()` sets no rule/file/range/suggestion) but never surface
via the CLI — defensive backstops for library callers, documented but
not pinnable (a small patch fits the stdin pipe even against an
instantly-exiting `git`; `wait` fails only on OS-level errors):
`failed to feed patch to git: <os error>` (stdin write failed) and
`` failed waiting for `git apply`: <os error> `` (child wait failed).

---

## 32. Non-Goals

RSmPL-Git v0.1 does not aim to:

1. replace normal Git patches;
2. be directly accepted by vanilla `git apply` when semantic hunks are present;
3. fully specify Rust macro expansion;
4. infer arbitrary refactors without explicit patterns;
5. perform global search without declared line ranges;
6. silently repair imports, visibility, or module paths;
7. guarantee type correctness unless semantic/type checking is explicitly enabled;
8. define a complete Rust parser grammar.

---

## 33. Versioning

The recommended versioning scheme is:

```text
RSmPL-Git MAJOR.MINOR
```

This specification is:

```text
RSmPL-Git 0.1-draft
```

### 33.1 Compatibility

Patch files may include an optional top-level version marker:

```text
# gitrs=0.1
```

However, to remain close to Git patch style, this marker is optional.

### 33.2 Future versions

Possible future extensions:

- multi-language support;
- typed semantic constraints;
- cargo-feature-specific symbol resolution;
- conflict markers for semantic patches;
- automatic import management;
- semantic move with path rewriting;
- explicit edition support (currently rejected, see Section 9.6):

```text
# edition=2021
```

---

## 34. Glossary

### AST

Abstract Syntax Tree. A structured representation of parsed Rust code.

### RSmPL

Rust Semantic Patch Language. A Rust-oriented semantic patch language inspired by Coccinelle SmPL.

### RSmPL-Git

A Git-like patch format that embeds RSmPL semantic hunks alongside ordinary unified-diff hunks.

### Semantic hunk

A hunk interpreted by AST or semantic matching rather than literal line replacement.

### Directive line

An unprefixed metadata line beginning with `#` immediately after a hunk header.

### Metavariable

A placeholder that binds to Rust syntax or semantic objects during matching.

### Ellipsis

The token `...`, used to match irrelevant syntax in defined positions.

### Capture

A named AST object bound by one hunk and reused by a later hunk.

### Lowering

The process of applying RSmPL semantic transformations and emitting a concrete ordinary Git diff.

---

## Appendix A: Canonical Minimal Syntax

The canonical RSmPL hunk shape is the diet minimal from Section 6:

<!-- rsmpl:invalid RSMPL_PARSE_CONFLICT -->
```diff
diff --git src/lib.rs
@@ -old_start,old_len @@ rule=name
# optional-directive
# metavariable-kind Name
- old_pattern
+ new_pattern
```

Notes: `diff --git <path>` alone suffices (the two-path form stays for
renames and Git-tooling compatibility); the `+new_start,new_len` side is
elided (RSmPL-only — it is ignored per Section 10.4, and ordinary hunks
must keep both ranges); `rule=name` is recommended for multi-hunk patches
but optional (a hunk without it is valid; diagnostics show `<unnamed>`);
`# lang=rust` is never required (detection is `rule=` or any `#` line)
and is omitted when other directives are present (Sections 7.1,
9.2). Metavariables are inferred — no `# kind Name` declaration line
in diet form (Section 15.8); the skeleton above names the position
only. Fences tagged `diff rsmpl` in Sections 28–29 are executable
under `spec_examples`; this skeleton is illustrative (placeholders)
and stays untagged.

The canonical ordinary hunk shape remains:

<!-- rsmpl:invalid RSMPL_PARSE_CONFLICT -->
```diff
@@ -old_start,old_len +new_start,new_len @@
 context
-removed
+added
```

---

## Appendix B: Required Behavior Summary

A compliant RSmPL-Git v0.1 implementation must:

1. parse Git-style file sections (a single `diff --git <path>` is allowed);
2. distinguish ordinary hunks from RSmPL hunks (elided new sides are RSmPL-only);
3. treat any `#` directive block as an RSmPL hunk (Rust by default; `# lang=rust` accepted, `# lang=<other>` rejected);
4. treat directive lines as metadata, not source lines;
5. derive file and range from Git patch structure;
6. restrict semantic matching to the declared old-side range;
7. support concrete Rust syntax matching;
8. support `...` in defined Rust syntactic positions;
9. support at least `expr`, `item`, `type`, and `pat` metavariables;
10. preserve the rule that modifiers are explicit;
11. apply ordinary Git hunks using normal unified-diff semantics when they do not conflict with semantic edits;
12. interpret all RSmPL old-side ranges against the original source coordinate system;
13. support forward capture references within the same patch file;
14. plan semantic edits before materializing final files;
15. fail rather than guess when validation fails;
16. lower the result to a concrete ordinary Git diff.
17. support struct/enum/trait/union/type/const/static/mod/impl-self-ty renames via bare `<kind> <Name>` pairs (one ident-span edit per match, all else preserved).

---

## Appendix C: Key Semantic Rule for `fn foo(...)`

This is the most important Rust matching rule in this draft.

The RSmPL pattern:

```rust
fn foo(...) {
    ...
}
```

matches:

```rust
fn foo() {}
```

and:

```rust
fn foo(x: i32) -> i32 {
    x + 1
}
```

It does not match:

```rust
pub async unsafe fn foo<T>(x: T)
where
    T: Clone,
{
    x
}
```

Reason:

```text
visibility: absent in pattern, present in code
qualifiers: absent in pattern, present in code
generic parameters: absent in pattern, present in code
where clause: absent in pattern, present in code
```

Modifiers and structural signature features must be explicitly written or explicitly abstracted.

---

## Appendix D: Agent CLI Surface (stdin, JSON report, ranges)

The patch format above is engine-agnostic, but the reference `applyrs`
binary (reached via the `gitrs apply` dispatcher) exposes agent-oriented conveniences. They do not change patch
semantics; they reduce the tokens and round-trips an LLM needs to author
valid `@@ -old,len` ranges and verify results. The tool is patching-only:
range discovery is a targeted file read (the agent's own tooling) plus
whole-file scope (D.3), not a CLI listing mode.

### D.1 Patch sources (stdin, `--file`, `--patch`) + `.history/`

No input flag means stdin (Unix filter default), so the cheapest
spelling needs no input flag at all:

```sh
# Run from the repo root; from elsewhere, add --repo ./my-repo to any command:
cat change.rsmpl | gitrs apply --check
```

`--file -` spells the same stdin read explicitly. Empty stdin is rejected
with the T59 `RSMPL_PARSE_CONFLICT` (empty input is an authoring typo,
never silent success). A bare interactive call with no pipe blocks on
stdin like any Unix filter — pipe or pass `--file`/`--patch`.

Three sources cover every case (T253 reverses the T135 `--patch` removal:
inline wins for agentic check loops — no `Write` round-trip — at the cost
of shell quoting and arg-length limits, so large patches keep `--file`):
 T259 audit KEEP: each source traces to a distinct token cost (bare stdin = cheapest pipe, `--file` = large patches, `--patch` = inline loops), so the trio is final; if a future cleanup must drop a flag, drop `--patch` (inline loops fall back to temp files) — never `--file`, never the bare-stdin default.

```sh
gitrs apply --file change.rsmpl --check          # large patches, no quoting
gitrs apply --patch "$(cat change.rsmpl)" --check # inline, no temp file
```

`--patch '<diff>'` conflicts with `--file` (clap usage error, enveloped as
`RSMPL_IO_CONFLICT` under `--format json`); an empty `--patch ""` is the
same empty-patch `RSMPL_PARSE_CONFLICT`. Canonical stdin spelling is
bare stdin (no flags); `--file -` is the explicit alias.

Every apply archives the raw patch text to `<repo>/.history/{id}.rsmpl`
(`<repo>` = the `--repo` value, default `.`; id =
`{unix_secs:010}-{nanos:09}-{pid:06}-{ctr:04}`, lexicographic order ==
chronological) and reports it on stderr as `history: <path>` plus the
additive JSON `"history"` key (Section D.2). `--check` skips the archive
unless `--history` is passed (clean measurement: a dry run leaves the
worktree untouched); a skipped archive reports JSON `"history": null`
and no stderr `history:` line. Only the
last 128 entries are kept (oldest pruned first). Archiving runs before
parsing, so failures leave audit too (when archiving is active); history IO faults are stderr
`warning:` lines only and never fail the run. `.history/` is untracked
audit noise (usually untracked; tracked/staged copies skip the same way),
never a lowering input (the lowering is assembled in-memory
from the old/new file contents; `git apply --check`/apply only ever
sees the lowered diff) — adding it to `.gitignore` is always safe
(the reference repo does).

### D.2 Machine-readable reports (`--format json`)

`--format text` (default) preserves the existing human output. `--format json`
prints one JSON object to stdout:

```json
{
  "version": 4,
  "rules": [{"rule": "rename_calls", "file": "src/lib.rs", "matches": 1, "spans": [[10, 12]], "semantics": "instantiated", "sites": [{"span": [10, 12], "bindings": {"E": "old_call"}}]}],
  "files": [{"path": "src/lib.rs", "added": 1, "removed": 1, "post_sha": "<sha-256 hex of the post-image>"}],
  "diff": "diff --git ... (lowered concrete patch, no RSmPL directives)",
  "diff_truncated": false,
  "diff_lines": 12,
  "diff_shown": 12,
  "checked": {"lowering": true, "semantics": "verified", "build": "skipped"},
  "history": "<repo>/.history/{id}.rsmpl"
}
```

- `rules` is the per-rule match report (text mode prints only one-line
  `applied`/`check` summaries; agents needing match counts use JSON).
  Each entry carries `semantics` (`instantiated` | `postcondition` |
  `reparse-only` | `not-applicable`, Section 24.6) plus `spans`:
  original-source 1-based inclusive
  `[start, end]` line ranges in application order, one per emitted edit, so
  agents can verify what matched without re-reading the file.
  Ident/call/cut/paste rewrites emit one edit per match
  (`spans.len() == matches`); signature-evolution replacements emit one
  span per edit (delete+insert pairs). Insertions report the anchor as
  `[L, L]`; an empty match set (optional no-match) reports `[]`.
  A `# follow=uses` hunk additionally carries one span per rewritten use
  site, so `spans.len() >= matches` there (`matches` still counts
  definitions only).   Cross-file uses (Section 14.8) report as extra
  entries under the same rule name — one per touched file, in sorted
  file order right after the definition entry: the definition entry
  keeps `matches` = definitions, each follow-only entry reports
  `matches` = its file's rewritten use-site count with `spans.len() ==
  matches` there and no `sites`.
  Each entry carries `sites` when any site binds: one
  `{span: [s, e], bindings: {...}, origin}` per emitted edit in span
  order (so `sites.len()` equals the number of binding sites, and each
  `span` repeats the corresponding `spans` entry). `origin` is carried
  provenance, never inferred from `semantics` (WI-15d): `"authored"`
  when the patch spelled the edit, `"inferred"` when the follow
  machinery added it (the parallel `span_labels` entry is `follow`;
  rules without labels read `"authored"`). Renames, insertions,
  deletions, and use-prefix renames bind nothing and omit the key
  (an empty map plus a zero count carries no evidence). A
  `# follow=uses` hunk with skipped sites carries hunk-level (no span)
  `follow_skipped_details: [{file, line, reason}]` (the v3 `; `-joined
  `follow_skipped` prose string was removed in schema `version: 4`;
  text mode prints one `follow-skipped: {file}:{line}:{reason}`
  line per skip). Values truncate at 80 chars (a cut value carries a
  trailing `…` so a truncated binding is never mistaken for a complete
  one) and entries at 8 (sorted order) per site; `sites_truncated` sums
  omitted entries across sites (absent when zero) and
  `bindings_truncated_chars` counts cut values (absent when zero). A
  cut value's site entry additionally gains `binding_meta: {name: {len,
  sha256}}` (`len` = full chars, `sha256` = hex of the full bytes;
  absent when nothing was cut), so cross-site binding equality is
  assertable without re-reading the file; `applyrs --bindings full`
  (also via `gitrs apply`) lifts every cap for that run only — all
  orphan entries and full texts, all site binding entries and full
  values (no truncation, no meta keys — the token-expensive path) —
  and any other `--bindings` value is an `RSMPL_IO_CONFLICT` usage
  error. Display never affects the verdict: `checked.semantics`
  reads pre-truncation engine counts, so default and `full` runs
  report identical `semantics` (WI-15b). Multi-edit rules whose spans
  (parallel to `spans`: one of `generics`/`params`/`return`/`rename`/
   `follow` per slot, `null` for unlabeled slots; omitted for
   single-edit rules, so identical spans like three `[1,1]`s stay
   distinguishable). Symbol hunks gain `symbol` (`"resolved"` when the
   Section 19.3 hook verified the seeds, `"syntax"` when
   `# fallback=syntax` carried the match; absent on non-symbol hunks),
   additive within `version: 3`, carried unchanged into `version: 4`, so an agent can require `"resolved"`
   for API renames, so
   the report stays within an absolute byte budget while an over-broad
  hunk still shows its extra span before apply. Budget (bytes beyond
  the reported `diff` string, `--diff-max-lines 0`): fixed envelope
  ≈90 B (`checked` + keys) + ≈80 B per `files[]` entry (64-hex
  `post_sha` dominates) + ≈200 B per `rules[]` entry (single-span,
   no bindings; includes the `semantics` word and the WI-15a totals) + ≈110 B per span (`[s,e]` plus per-site `{span,
   bindings, origin}` evidence — that growth is evidence, not overhead) + at most
   8×83 B per binding site (80 chars plus the 3-byte `…` marker when cut)
   + ≈110 B per cut value (the `binding_meta` digest; measured ≈91 B,
   pinned at 110). Operational pin:
   overhead ≤ `200 + 150·F + 240·R + 115·S + 110·C` bytes (`F` = files,
   `R` = rules, `S` = total spans, `C` = cut values). The `240·R`
   prices the three WI-15a totals keys (≈60 B) and `115·S` the
   per-site `origin` (≈21 B). There is deliberately no relative
  % bound: a sub-KB report can exceed any % while staying within
  bytes, and a multi-KB report is where the pin applies.
- `files` lists per-file `{path, added, removed, post_sha}` computed from
  the full old/new contents, so counts stay exact even when `diff` is
  truncated or omitted (see D.2.1). `post_sha` is the SHA-256 hex of the
  materialized post-image. Empty when no changes. `files[].path`
  is the single source of changed paths (a former `files_changed` key that
  duplicated these paths was removed as pure report overhead). The text
  apply `applied N file(s): …` stderr summary reports this same
  diff-based set (old-vs-new contents differ); the engine reports only
  per-rule matches and owns no separate changed list.
- `diff` is the lowered ordinary Git diff (empty string when no changes).
- `diff_truncated` is `true` iff the lowered diff is non-empty and the
  reported `diff` string hides any of it (`--diff-max-lines N` cut lines
  off, including `N=0` which omits); else `false` (including no-changes).
  `diff_lines` is the line count of the full lowering (`0` when no
  changes); `diff_shown` is the line count of the reported `diff` string
  (`0` when omitted or no changes). JSON consumers size the next budget
  as `diff_shown` of `diff_lines` instead of guessing (see D.2.1).
- `follow_files`/`follow_source` report the workspace search set
  (Section 14.8): the count of `.rs` files in the set and `"git"`
  (tracked + untracked-but-not-ignored) or `"walk"` (lexical fallback
  outside a worktree). Present whenever the patch needed a workspace
  search (`# follow=uses` rewrites or plain-rename orphan scans);
  additive keys that do not bump `version`.
- `drift` reports stale-range retries (Section 10.8): one
  `{rule, file, cited: [s,e], actual: [a,b]}` per drifted hunk in
  application order (anchored and pattern-anchored tiers emit the identical
  shape), absent when no hunk drifted. Text mode prints one
  `drift: {rule} {file} cited [s,e] -> actual [a,b]` stderr line per
  entry (both `--check` and apply); JSON mode carries the evidence in
  the payload only. Carried unchanged from `version: 3` into `version: 4`.
- Per-rule `orphans` reports definition-only rename fallout
  (Section 14.8): `[{file, line, reason, text}]` (`reason` is the WI-2
  vocabulary: `off-target` for uses left behind, `macro-hidden` for
  opaque interiors) plus `orphans_truncated`
  when more than 8 lines name the old ident (each `text` truncated to
  80 chars with a trailing `…` when cut; `--bindings full` lifts the
  entry cap and the cut here too) plus `orphans_truncated_chars`
  counting cut texts (absent when zero); absent when no use names the
  old ident. The always-present WI-15a `orphans_total` counts the
  pre-truncation engine total (shown plus `orphans_truncated`, or all
  shown under `full`). Carried unchanged from `version: 3` into
  `version: 4` (like the other per-rule evidence keys; `version: 2`
  added them within v2).
- `version` is the JSON report schema version (`4` = this contract:
  `checked.semantics` word (`verified`|`partial`), per-rule `semantics`
  words; `v3` added the `semantics` words and evidence keys over the v2
  boolean `semantics`; `v2` added them over `v1`'s boolean `check`,
  first-site `bindings`, and `verified`; `v4` removes the `follow_skipped`
  prose string and unifies `diffrs` on the same number). Agents
  pin parsing on it; any future breaking JSON change bumps it
  (additive keys like `history`, `post_sha`, `sites_truncated`,
  `bindings_truncated_chars`, `follow_files`, `follow_source`,
  `orphans`, `orphans_truncated`, `orphans_truncated_chars`,
  `orphans_total`, `follow_skipped_total`, `sites_total`, per-site
  `origin`
  do not bump). All three binaries (`applyrs`, `diffrs`, `gitrs`
  dispatcher errors) take the number from the single
  `rsmpl_patch::REPORT_VERSION` constant — never per-binary literals —
  so the `gitrs` sibling version-skew handshake transitively pins
  schema parity.
- `checked` is the trust envelope: `{lowering, semantics, build}` on
  every successful run, each computed by a check that can fail on real
  input (never true by construction). `lowering` is the `git apply
  --check` verdict on the full lowering (empty lowerings skip git and
  report `true`);   `semantics` is `"verified"` iff every rule is
  `instantiated`/`postcondition`, no ordinary hunk is present, every
  rule's engine orphan count is zero, every rule's follow-skip count is
  zero, and no rule's evidence was truncated (orphan cap 8 / orphan text
  80 chars / site bindings cap 8 / binding value 80 chars all unhit —
  counts read before JSON truncation), else `"partial"` (Section 24.6:
  unverified insertions, cuts, pastes,
  member-additions, unverified
  follow-only extras, optional zero-edits, ordinary hunks, any orphans,
  any follow-skips, and any truncated evidence all force `partial`).
  Text mode names blockers with one
  `incomplete: rule <name> (<file>): <n> orphans, <m> follow-skipped[, truncated]`
  line per blocking rule (plus `incomplete: ordinary hunks present`
  when ordinary hunks block and `incomplete: rule <name> (<file>):
  semantics <word>` when the rule word itself blocks); JSON stays
  stdout-only with no such lines. This completeness clause is a breaking
  meaning change of `verified` (v5; the version bump lands in its own
  item, the derivation lands here);
  `build` is `"pass"` iff `--verify build`
  ran, else `"skipped"` (a build failure exits 2 with the `{error}`
  envelope below, never a success payload). Failures never reach this
  payload (exit 2 with the `{error}` envelope below).
- `--verify build` (opt-in, never by default) additionally runs
  `cargo check --quiet`. Without `--check` it runs in `--repo` after a
  successful apply; failure exits 2 with the first `error` diagnostic
  line in the reason and the patch stays applied (JSON carries
  `"applied": true`). With `--check` it materializes the in-memory
  post-images into a temp copy of `--repo` (tree copied minus `.git`,
  `target`, `.history`; symlinks recreated, never followed) and runs
  `cargo check --quiet` there — under `--check`, verifies in a temp copy
  and reports `applied: false` on failure; the worktree stays
  byte-identical and failure exits 2 with `"applied": false`. When the report carries
  `orphans`, the reason appends the first orphan `file:line` entries
  in brackets. Success reports `"build": "pass"` inside `checked`.
  Empty lowerings skip verification. Any other `--verify` value is an
  `RSMPL_IO_CONFLICT` usage error.
- `history` is the `<repo>/.history/{id}.rsmpl` archive path of the raw
  patch input (`null` when archiving warned or when a `--check` run
  skipped the archive without `--history`), reported alongside the
  stderr `history:` line when present (Section D.1).
- The v1 `check` boolean is gone: `--check` vs apply never changes the
  report shape (the flag chooses check vs land, `checked` reports the
  gates either way).
- `rules` follows patch document order (file-section order, then
  hunk order within a section), so a multi-hunk patch reads
  top-to-bottom like the patch itself — including move cut/paste
  hunks, which the engine evaluates in a different pass but
  reports in document position. `rule=` names need not be unique:
  each hunk reports its own entry, and same-named hunks are told apart
  by `file`/`spans`.
- `files` follows patch file-section order.
- `--output <path>` writes the full diff file in both modes, even when
  the reported `diff` is truncated or omitted — with an empty lowering it
  instead removes a pre-existing file at that path and otherwise creates
  nothing, so the path is present iff the lowering is non-empty (never a
  stale diff from a previous run). It is written before
  `--check` runs, so a failing `--check` still leaves the full appliable
  diff for debugging (see D.2.3). Missing parent directories are created
  automatically; unwritable targets stay `RSMPL_IO_CONFLICT` (25.8).
  The path is resolved against the caller cwd, never joined to `--repo`
  (see 27.6).
  Shell redirection does not replace it: text `--check` stdout is the
  diff and is redirectable, but with `--format json --diff-max-lines 0`
  stdout is the JSON report, not the diff — only `--output` saves the
  full appliable lowering while the preview stays truncated.
- Exit codes are unchanged: `0` applied/clean, `2` patch error with a
  structured `RSMPL_*`/`GIT_*` diagnostic on stderr.
- Defaults audit: `--format text`, `--repo .`, and `--context 0` stay;
  the verified lowering is always 0-context (minimal tokens by default,
  still appliable via the `--unidiff-zero` wrapper; `--context N` only
  renders presentation context around the same changes, D.2.2). Text
  keeps human review working (JSON is a ~4-token opt-in);
  `--repo` defaults to `.`, so run from the worktree root and omit it,
  adding `--repo DIR` only when invoking from elsewhere.
  When DIR is a subdirectory of the work tree, the CLI runs `git apply`
  at the enclosing root with `--directory=<prefix>`, so subdir invocation
  applies identically (plain CWD-relative resolution would silently skip).
- Sibling handshake: `gitrs` reads each sibling's `--version` before
  re-exec and fails with `RSMPL_IO_CONFLICT` naming both versions on
  mismatch (same stdout-only JSON envelope with `--format json`);
  `GITRS_*_BIN` overrides (test doubles) skip the check. All three
  binaries report the same `rsmpl_patch::REPORT_VERSION` schema number,
  so the handshake transitively pins schema parity: skewed siblings
  cannot silently disagree on the report shape.
- Failure envelope: with `--format json`, every patch/IO failure prints
  one JSON envelope to stdout and exits 2, so an agent can repair without
  parsing stderr prose. The failure is stdout-only: no stderr `error:`
  line is printed in JSON mode (T89 symmetry with the silenced success
  `check:` line); stderr carries `warning:` lines only. Text mode keeps
  the stderr diagnostic.

```json
{"version": 4, "error": {"code": "RSMPL_MATCH_CONFLICT", "rule": "r", "file": "src/lib.rs", "range": "1..2", "reason": "no match found", "suggestion": "read `src/lib.rs` to list valid ranges; check the pattern or add `# optional`"}}

```

- The envelope always carries `version` plus all seven error keys (`code
  rule file range reason suggestion repair`); absent optionals are `null` (so `repair` is
  `null` on errors that carry no machine payload). `code` is one of the SPEC 25
  codes, including `RSMPL_IO_CONFLICT` (25.8) for unreadable patch, repo,
  or output files, stdin failures, and flag-usage errors, and
  `RSMPL_BUILD_CONFLICT` (25.10) for post-lowering `--verify build` failures.
- `repair` is the machine-readable self-correction payload (WI-10 direction,
- additive under the `version` rule above: text mode prints it as a trailing
- `repair: {...}` line only when present). Completeness rule (WI-3): every
- error class reachable from patch input carries a `repair`, so no patch
- failure is prose-only; the allow-list entries are the unreadable
- patch file (`RSMPL_IO_CONFLICT` with null `repair` — environmental, the
- `suggestion` names the path instead; 25.8), the build-environmental
- failures (missing `Cargo.toml` / missing `cargo`, same code; 25.10),
- and the build double-miss (location-less failure with no orphans;
- 25.10). A new null-`repair` error fails the
- `wi3_repair_complete_e2e.rs` enumeration. CLI argv errors
- (bad flags) are outside the enumeration — not patch-input reachable —
- and keep their suggestion-only envelope.
- Zero-match with exactly one whole-file match (25.4): `{"action":"widen_scope",
- "directive":"# scope=file","range":"@@ -1,1 @@"}` — apply the directive +
- range verbatim and retry (mechanical first-retry, no prose parsing).
- Zero-match otherwise (25.4): `{"action":"read_file","file":F,"range":R}`
- with `"candidate":T67-excerpt` when a nearest candidate exists (verbatim
- string in v0.1; a structured `{line,text}` split is WI-3c).
- Too-many (25.4): `{"action":"add_directive","directive":"# allow=many"}`.
- Build failure under `--verify build` (25.10): `{"action":"read_file","file":F,"range":"L..L"}` —
- the first cargo diagnostic wins, else the first orphan; null only on the double-miss (allow-list).
- Range past EOF (25.1; ordinary-hunk EOF shapes, 25.7): `{"action":"fix_range",
- "file":F,"file_lines":N,"write_instead":"@@ -s,l @@"}` — the largest
- in-file `@@` range starting at the cited start (insertion anchors clamp
- to the end anchor `@@ -N+1,0 @@`; the cited start past EOF clamps to the
- last line). Paste the header verbatim and retry. One shared clamping
- rule feeds both the range validator and the ordinary-hunk applier.
- Unknown directive / header tag / `# root=` value (25.2/25.3):
- `{"action":"fix_directive","found":F,"did_you_mean":D,"write_instead":W}` —
- `found` echoes the offending spelling, `did_you_mean` the closest known
- spelling or `null` (honest when nothing is close), `write_instead` the
- full corrected directive line (or a removal naming the legal options)
- to paste verbatim and retry. Key suggestions gate on the same first
- character plus edit distance ≤ 2 (`# alow=many` → `# allow=many`;
- `# foo=bar` suggests nothing although `foo` is distance 2 from `root`).
- Value suggestions over the closed sets are ungated (`# scope=files` →
- `# scope=file`); a far miss lists every legal spelling instead.
- Pattern-shape (18.6 C-table): `{"action":"fix_pattern","code":"C1"-"C20",
- "write_instead":"<SPEC 18.6 text>"}` — `code` is the C-table row, `write_instead`
- the exact diagnostic telling the agent what to write instead (mechanical:
- replace the rejected spelling with `write_instead`, retry). Gates outside
- the C-table (unknown `# root=`, unsupported `# <kind>`, `# attributes`,
- `# repeat` misuse, `unbound` metavariables, `ellipsis`, `tokens`,
- `format`, `bind`/`use` shape errors) use the same shape with a gate-name
- `code` (`"root"`, `"kind"`, `"attributes"`, `"repeat"`, `"unbound"`,
- `"ellipsis"`, `"tokens"`, `"format"`, `"bind"`, `"use"`, `"pattern"` for
- the long tail); the engine boundary fills any `PatternConflict` without a
- specific code as `"pattern"` with a keyed concrete template (WI-2b hygiene:
- never the echoed reason), and any `CaptureConflict` as `"capture"` likewise.
- Symbol/capture: symbol gates (C13, follow veto, unparseable tripwire)
- carry `{"action":"add_directive","directive":"# fallback=syntax"}` (apply
- verbatim, retry); unbound/duplicate captures carry
- `{"action":"fix_name","name":N,"available":[...]}` (rename to a listed
- capture, or add a `# bind=N` hunk when empty).
- Ordinary-hunk count mismatch (T367, 25.2): `{"action":"fix_header",
- "expected_old":N,"expected_new":M,"claimed_old":A,"claimed_new":B,
- "expected_header":"@@ -s,N +t,M @@"}` — `N`/`M` are the counted body lines
- (old-side = ` ` + `-` lines, new-side = ` ` + `+` lines), `A`/`B` the claimed
- header counts, and `expected_header` the drop-in corrected header line.
- When the body was truncated by an unprefixed line (WI-1b suspect arm),
- the same action carries `"expected_header":null` plus
- `"suspect_line":N,"suspect_text":"...","fix_line":"prefix line N ..."` —
- `N` is the 1-based patch line that never entered the body; prefix it
- with its marker and recount instead of pasting a header. Headers,
- blank separators, metadata, and EOF are never suspects (pure miscount).
- Ordinary-hunk content mismatch (25.7): `{"action":"fix_context","line":N,
- "expected":"...","actual":"..."}` — `N` is the 1-based original-file line,
- `expected` the hunk's line text, `actual` the file's line text. Full
- single-line texts, never truncated: paste `actual` over the hunk line (or
- vice versa) to self-correct first-try without re-reading the file.
- Flag-usage errors (unknown flag, bad value) are enveloped too: with
  `--format json` they print the same stdout-only `RSMPL_IO_CONFLICT`
  envelope (reason = clap's one-line error, suggestion = run
  `applyrs --help` for usage) and exit 2. Text mode keeps clap's stderr
  error. (No-input is not an error: bare stdin is the default, D.1.)
  One caveat: a missing `--format` value cannot be detected as a json
  request, so it always takes the text path.

#### D.2.1 Output budget (`--diff-max-lines N`)

One large move can produce a lowered diff that exceeds an agent's context
all by itself. One apply-path flag bounds only the *reported* diff; it
never changes what is checked, applied, or saved:

```sh
# Run from the repo root; from elsewhere, add --repo ./my-repo to any command:
# Stats + check only, no diff tokens at all:
gitrs apply --file change.rsmpl --format json --diff-max-lines 0 --check

# First 40 lines of diff for a quick skim, stats stay exact:
gitrs apply --file change.rsmpl --format json --diff-max-lines 40 --check

# Text equivalent (stdout is the reported diff, markers on stderr):
gitrs apply --file change.rsmpl --check --diff-max-lines 40
```

Rules:

- `--diff-max-lines N` keeps the first N lines (`0` omits). A truncated
  diff is review-only: it is not guaranteed to `git apply`.
- `diff_truncated` (JSON) is `true` iff the lowered diff is non-empty and
  the reported `diff` hides any of it; the marker is exact, not heuristic.
- Text `--check` prints stdout as pure diff-or-empty (untruncated output
  still pipes to `git apply`); budget markers go to stderr: `diff omitted
  (--diff-max-lines 0): {n} file(s) changed` plus per-file
  `  {path}: +{added} -{removed}` lines, or `diff truncated: showing
  {shown} of {total} lines (full diff in --output file if given, else
   re-run without --diff-max-lines)`. Plain text apply prints nothing to
   stdout (only `applied ...` on stderr); an empty lowering prints
   `(no changes)` to stdout in text mode (JSON returns the report
   silently — see D.2.3).
- `--check`, apply, and `--output` always use the FULL lowered diff, so
  `--format json --diff-max-lines 0 --check` verifies clean application
  with zero diff tokens, and `--diff-max-lines 1 --output full.patch`
  saves the appliable patch while showing one line. With no changes the
  flag is a no-op (`"diff": ""`, `"diff_truncated": false`, `"files": []`).
- The lowering itself is always 0-context (D.2.2): the budget flag above
  applies *after* lowering.

#### D.2.2 Lowering context (verified 0, presented N)

D.2.1 bounds the *reported* diff; the *verified* lowering is always
0-context (minimal tokens by default). `applyrs --context N` (default
0, canonical form unchanged) additionally renders the same changes with
N context lines for the reported/`--output` diff:

```sh
# Run from the repo root; from elsewhere, add --repo ./my-repo to any command:
# Minimal tokens by default, still a real appliable patch:
gitrs apply --file change.rsmpl --check
# Vanilla-compatible artifact (no --unidiff-zero needed to apply it):
gitrs apply --file change.rsmpl --check --context 3 --output full.patch
```

Rules:

- The verified lowering shows only changed (`-`/`+`) lines; hunk headers
  stay `@@ -a,b +c,d @@`. applyrs's own `--check`/apply path handles
  it (it passes `git apply --unidiff-zero`, which plain `git apply`
  requires for zero-context hunks); piping a saved 0-context patch to a
  bare `git apply` needs that same flag.
- `--check` and apply always use the 0-context lowering; the reported
  `diff` (`diff_lines`/`diff_shown`, text `--check` stdout) and
  `--output` carry the `--context N` rendering of the same post-images.
  A `--context 3 --output` artifact applies under vanilla `git apply`.
  A truncated report (`diff_truncated: true`) is review-only either way.
- Per-file `{path, added, removed, post_sha}` stats are computed
  from the full old/new contents, so they stay exact under any report
  budget and any `--context` (the post-tree is identical; only the
  rendered diff text varies).
- Layering: the D.2.1 report budget applies after rendering.
  `--diff-max-lines 2` shows the first 2 lines of the rendered diff;
  `--diff-max-lines 0` omits with `diff_truncated: true` iff
  changes exist. With no changes the budget flag is a no-op.
- Four-lever summary: `--format` chooses the report language (text diff
  vs JSON object), `--context` chooses the rendering radius (0 verified
  minimal vs N vanilla-compatible), `--diff-max-lines` trims only the
  reported `diff` preview, and `--output` always saves the full
  rendered diff — so `--format json --diff-max-lines 0 --check
  --output full.patch` verifies with zero preview tokens while keeping
  the appliable patch on disk.
- Unknown `--context` values are a clap usage error: plain text by
  default, `RSMPL_IO_CONFLICT` envelope with `--format json` (usage
  errors are enveloped, D.2).

#### D.2.3 Output streams and empty lowerings

Agent loops pipe stdout (to `git apply`, to a JSON parser) and must know
exactly which bytes land on which stream. The contract, per mode:

| Mode | stdout | stderr |
|---|---|---|
| text apply | empty (quiet by default) | `history: …` + `applied N file(s): …; semantics verified|partial` (kept: one line of human reassurance) |
| text `--check` | reported diff (empty when omitted/no changes) | (`history: …` only with `--history`) + budget markers + `check: lowered patch applies cleanly; semantics verified|partial` |
| JSON apply | report object (incl. `"history"`) | `history: …` + `warning:` lines only (failures live in the stdout envelope, no stderr `error:` line) |
| JSON `--check` | report object with `"checked"` (incl. `"history"`, `null` unless `--history`) | (`history: …` only with `--history`) + `warning:` lines only (no `check:` line: stdout is the signal; failures likewise stdout-only) |

Rules:

- An empty lowering (no changes) never touches git: `git apply
  --check` is skipped and the run reports clean. Text prints
   `(no changes)` to stdout; JSON returns the report silently
   (`"diff": ""`, `"diff_truncated": false`, `"files": []`, v2
   `"checked"` object).
   `--output` mirrors the current lowering: a non-empty lowering writes
   the full diff, while an empty lowering removes a pre-existing file at
   that path and otherwise creates nothing — the path never holds a stale
   diff from a previous run. An unremovable stale file fails loud with
   `RSMPL_IO_CONFLICT` (exit 2) before `--check`/apply runs, engine-verbatim:
   `reason: cannot remove stale <path> (empty lowering): <os error>`
   with `file:` echoing the `--output` path (text omits the null
   `rule:`/`range:`/`suggestion:` lines, JSON carries explicit nulls;
   the `<os error>` tail is platform-dependent, like the sibling
   `cannot write <path>: <os error>` shape in Section 25.8).
- `--output` is written before `--check` runs, so a failing `--check`
  still leaves the full appliable diff on disk for debugging.
- A truncated report (`--diff-max-lines N` cut lines off) is
  review-only: never pipe truncated stdout to `git apply`. Re-run
  without the flag or take the full diff from `--output`.
- `warning:` lines (`… treating as new (empty) file`,
-  `history: cannot …`) go to stderr in both modes. A `--format json` consumer parsing
  stdout must also read stderr, or it will miss them. The `history: …`
  archive line goes to stderr in all modes (stdout stays pure diff-or-report).

#### D.2.4 Repo-free patch lint (no new flags)

Authoring check without a checkout: parse, directive validation,
pattern compilation, and the capture graph (`# bind=`/`# use=`,
plus-side binding) all run before matching, so an empty directory
stands in for the repo:

```sh
mkdir -p /tmp/empty && gitrs apply --file change.rsmpl --repo /tmp/empty --check
```

Triage the result: `RSMPL_PARSE_CONFLICT` /
`RSMPL_PATTERN_CONFLICT` / `RSMPL_CAPTURE_CONFLICT` (and symbol
errors) are real authoring bugs — fix them. `RSMPL_RANGE_CONFLICT`,
`RSMPL_MATCH_CONFLICT` (`no match found`), and `GIT_*` failures are
empty-dir artifacts: the empty file counts as 1 line (authored
ranges exceed it), there is no code to match, and insertion
lowerings cannot `git apply` there — ignore them and re-check
against the real repo. In particular `no match found` on the empty
dir is the lint-pass signal: the patterns compiled and the captures
bound.

One caveat: range validation (Pass 0) runs before the
pattern-body/capture/symbol stages, so authored `@@ -old,len` ranges
mask those stages on the empty dir (the first range error wins) —
but not directive validation, which is parse-time and runs before
Pass 0, so an unknown `#` directive surfaces regardless of range.
To lint the deeper stages, temporarily lint with the `# scope=file` +
`@@ -1,1 @@` dummy (17.3) on each hunk — the dummy skips range
validation and the body/capture/symbol errors surface. Expect the
stderr `treating as new (empty) file` warning (D.2.3); insertion-only
patches always need the real repo for the final check. No `--lint`
/ `--no-repo` flag is needed, so the T34 lint gap closes with zero
flags added.

### D.3 Range discovery (file reads + whole-file scope)

The CLI lists nothing: range discovery is a targeted file read with the
agent's own tooling. Read the target file, note the 1-based inclusive
spans of the items to rewrite, and cite `@@ -start,len` old-side ranges
with `L = end-start+1` (e.g. a `fn foo` on lines 1-3 → `@@ -1,3 @@`).

When the agent already holds a small file, skip discovery entirely:
`@@ -1,1 @@` + `# scope=file` searches the whole file (17.3) with
zero line counting.

Agent workflow detail lives in `skills/gitrs/SKILL.md`.

Recommended agent loop:

```text
read file -> author hunks in original-source coordinates
-> apply with --format json --diff-max-lines 0 --check -> review files[] stats
-> widen with --diff-max-lines N / --output full.patch only if needed
-> apply for real
```

New authors: start with `AGENTS.md` (one-page authoring reference); this
appendix and Sections 9/18.6 are the normative detail behind it.

### D.4 Inverse mapping (`diffrs`: native diffs → RSmPL)

`gitrs diff` (sibling `diffrs` binary) inverts the authoring
direction: it reads pending native changes (or history) and prints a
compressed RSmPL-Git patch to stdout — the token-efficient view of
what changed. Minimal surface, same philosophy as `applyrs`:
`--repo` (default `.`), `--format text|json` (default `text`),
`--staged` (index vs HEAD/rev instead of worktree), one optional
`[rev]` forwarded to `git diff [rev]` (e.g. `HEAD~1` for history),
`--patch <TEXT>` / `--file <PATH>` to raise from native patch text
instead of collecting (e.g. `git diff` output, a saved PR patch — the
repo supplies the pre-image, the text supplies the delta; `--staged`
/ `[rev]` still select the pre-image base either way; nothing is
written to the worktree), and `--check` to verify the raise
in-process (below).

Collection: unstaged worktree-vs-index plus untracked files (emitted
as new-file ordinary sections, since `git diff` never shows them;
untracked symlinks read as their link-target string, never followed,
with `new file mode 120000`); untracked embedding is pending-worktree
scope only — `--staged` and `[rev]` conversions exclude untracked
files, exactly like the native `git diff --cached` / `git diff [rev]`
forms they mirror (T392). `--repo` may be a subdirectory — reads
resolve to the worktree top level while output paths stay
repo-root-relative (check/apply run against the root, T390).
empty files emit headers only (no hunk, exactly real git's shape);
untracked executables emit `100755` (unix) so +x round-trips (T364);
`--staged` selects index-vs-HEAD/rev; `[rev]` covers history with the
same mechanism, including ranges (`A..B` / `A...B` diff endpoint
against endpoint). A range resolves its pre-image base to its left
side (`A`): the range string itself never resolves in `git show`, so
without the split every detector would silently downgrade to
section-visible totality. A staged/rev conversion whose base lacks a
changed `.rs` file warns per path on stderr (`no pre-image at the rev
base; provenance downgraded to section-visible`) instead of weakening
`proven` silently. `git diff` runs with `--no-color --no-ext-diff
--unified=0` (rename detection stays on: moves surface as `rename
from/to` sections that round-trip through `applyrs`, T377).
Context radius is 0, so
ordinary passthrough carries no untouched context lines yet still
round-trips. CRLF file content passes through byte-exact (the `\r`
rides the body lines; semantic detectors decline `\r`-carrying
sections, T385). Binary files are
stripped with a stderr `warning:` per path (a patch is UTF-8;
`applyrs` cannot read binary worktree files); untracked binaries emit
the same marker sections `git diff` shows once staged, so they warn
uniformly instead of vanishing silently, and an all-binary change
reports empty + warnings (exit 0), not an error (T378). Sections whose
`+`/`-` body lines carry NUL bytes strip the same way (git's binary
definition); invalid-UTF-8 sections (native `git diff` dumps literal
bytes for binary creates while modifies get the marker) are swapped
for `Binary files … differ` markers at the byte level, so genuine
U+FFFD chars in valid-UTF-8 text pass through (T395/T396). Emitted
untracked headers and marker slots C-quote `"`, `\`, control bytes
(native quotepath shape; plain names stay raw) — raw interpolation
mis-split quote names into silent wrong-file writes (T399). Submodule
(gitlink, mode 160000) sections strip the same way (one `warning:`
per path): a gitlink bump/delete cannot round-trip (`applyrs`
cannot read the directory on the pre-image side) and an add only
materializes an empty dir, while real git needs submodule machinery
no patch carries (T386). Untracked directory entries (nested repos,
empty dirs — real `git diff` hides them) skip the same way instead
of failing the conversion (T387). Top-level `.history/`
audit files (tracked, staged, and untracked) are skipped the same way
(one `warning:`; they are tool
output, never lowering inputs). Git's trailing
`@@ -a,b +c,d @@ <funcname>` section titles are dropped (RSmPL
headers admit only `rule=` tags, Section 8.3; `git apply` likewise
ignores them), as are `index <sha>…` lines (optional file metadata
that `applyrs` parses but the lowering never reproduces — T374).
Two-path `diff --git a/P b/P` headers collapse to single-path
`diff --git P` (SPEC 6 equivalence, T381 — 6 + len(P) bytes saved per
section); renames, `/dev/null` halves, and quoted headers keep
two-path, and `---`/`+++` markers stay (bare `git apply` rejects
stripped markers).
Headers-only sections (mode flips, empty creates: no `---`/`+++`
markers) keep two-path — bare `git apply` reads filenames from the
markers and fails on a collapsed single path without them (T393).

Inference v1 is pure: a file section whose every pair shares one
shape is fully replaced by one semantic hunk (this preserves
cross-hunk sweeps); otherwise each balanced native hunk retries the
same detectors independently (T349) and leftovers stay ordinary in
original order — native hunks are pairwise disjoint in original
coordinates, so a compressed hunk never overlaps a leftover hunk
(Section 10.6 option-1 merge by construction). Emission range rule
(enforced, Section D.5): when all sites of one detector class lie
within one contiguous original range of at most 64 lines, emit that
tight real range with `# allow=many` and no `# scope=file`; otherwise
emit the `# scope=file` + `@@ -1,1 @@` dummy (Section 17.3).
Expression/line detectors (call, method, use-prefix, verbatim,
peel/literal section drivers) cite the changed minus lines directly —
their matches are single-line, so the minus span is the match span.
Sub-line pairing (WI-4): when no pair of a section parses whole-line
but every pair parses as one expression holding the same count (≥1) of
one family's subterms (bare calls, method calls, or qualified calls)
that pair in order as that family's delta (callee-only, method-only,
or last-segment-only, per-site args/receivers token-equal, one shared
rewrite within the line), and the plus line equals the minus line
under sequential subterm replacement (any frame delta or unchanged
same-line twin vetoes), the pair contributes its subterms as sites
instead of its lines. Frames unwrap to their inner expression before
classification, tried only after whole-line parses fail (WI-4, widened
WI-3): a single-statement one-line `fn` frame (a changed line parsing
as a `syn::ItemFn` whose body holds exactly one statement contributes
that statement's expression — `Stmt::Expr` with or without `;`, or a
`let`-init with `let-else` declined); a `let` whose init is a closure,
`async` block, or bare block holding exactly one statement (which
contributes its expression; bare closure bodies contribute
themselves); a one-line `match` arm body after stripping the unchanged
arm head textually at the first `=>`; a one-line `if`/`else` branch
body after stripping at the first `{` (the head must name `if` or
`else`). Multi-statement bodies, macro bodies, and item declarations
never unwrap and stay ordinary. Emitted hunks carry no frame text —
the frame is byte-identical on both sides so the subterm rewrite
reproduces the post-image. Frame equality is proved per pair on the
full line text: the plus line must equal the minus line under
sequential subterm replacement of the extracted (rendered) subterms —
any frame delta (renamed `fn`, rebound closure parameter, changed arm
pattern, changed branch keyword, changed `let` wrapper) or unchanged
same-line twin vetoes. The same per-pair check guards the whole-line
general-expression sweep (framed lines included) and the control
positions (guard, arm body, closure body, condition, iterable,
diverge, await/`?` base), whose templates likewise rewrite only the
subterm. Totality then counts occurrences of the minus
subterm in the pre-image (engine-count probe, as verbatim does), not
changed lines — two calls per line count twice. The emission is the
same bare-subterm hunk as the whole-line shape. Single-pair sections stay ordinary for
call/method/use shapes even with two sites (T375: one pair never wins
as a sweep) — except whole-arg permutation with ≥ 2 subterm sites
(WI-6 slice 2, `call_arg_template`): `swap(x, y) + swap(y, x)` →
`swap(y, x) + swap(x, y)` re-infers `swap(E0, E1)` → `swap(E1, E0)`
because the ordinary view cannot show multiset equality while the
semantic form proves the permutation intent; fn/item renames and moves
keep firing at N = 1.
Item-shape detectors (item rename, slot sweep) match whole multi-line
items, so a header-line range could never contain the match: they emit
the covering item range (smallest `syn` item spans containing the
sites, attrs/docs included) when the caller supplies a parseable
pre-image, else the dummy. Moves and deletes keep today's real
ranges/anchors. Rule names are `r1…` in canonical hunk order.
Sections carrying `old mode`/`new mode` skip the
whole-section attempt and go straight per-hunk — whole-section
emissions rebuild only the `diff --git` header and would silently
drop the mode flip, while the per-hunk prelude keeps the mode lines
(T401).

`diffrs --format json` (default `text`) prints `{"version": 4, "files":
[{path}], "patch": "<RSmPL-Git patch>", "hunks": [...]}` to stdout
(text mode prints only the patch). Failures carry the same error
object as `applyrs` (`code/rule/file/range/reason/suggestion/repair`
via `StructuredError`, stdout-only with `--format json`, exit 2).
`--check` verifies the raise with the T361 loop: the raised patch is
scratch-applied to an in-memory copy of the pre-image and
byte-compared against the true post-tree derived by applying the
native diff to the same pre-image (changed-files bounded; no worktree
copy, no `git apply`). JSON gains `"check": {verified,
files_compared, first_divergence}` (`first_divergence` is `null` on
success, else `{file, line, want, got}` naming the first differing
line — a failure is never a bare `false`); text mode keeps stdout as
the patch artifact and prints a one-line `verified: true|false …`
verdict on stderr. Failure exits 2 with the divergence named. The
check never trusts lowering validation alone: `applyrs --check`
(parse + lower + `git apply --check`) stays green on sweep
over-match — the lowered diff applies cleanly while rewriting sites
the native diff never touched — so only the scratch-apply +
byte-compare proves a raise (T361). Provenance boundary: `--check`
proves changed-files closure; workspace intents (`# follow=uses`)
additionally search off-diff workspace files that mention a followed
ident (one `git grep` over tracked files at the diff base plus an
in-process scan of untracked pending-scope files) — a mentioning
file joins the D13 probe domain, so an intent whose site set exceeds
the native diff declines to ordinary at raise time instead of relying
on the engine's apply-time backstop. Mention-free workspaces raise
byte-identical to the changed-files-only domain.
Each `hunks` entry carries per-hunk
provenance for one emitted semantic hunk, plus one entry per
wholly-ordinary section (WI-15c — the list is never bare `"hunks":
[]` while sections went unexplained):
`{rule, file, detector, sites, sites_exact, totality, proven,
saved_bytes, group}` plus `declined` on ordinary entries only.
`detector` is
the firing detector family (`item_rename`, `fn_slot_sweep`,
`method_to_fn`, `method_to_method`, `method_same_literal`,
`method_arg_template`, `call_arg_template`, `general_expr`,
`ref_strip`, `qualified_call`, `turbofish`, `macro_rename`,
`use_prefix`, `call_sweep`, `call_arg_add`,
    `call_literal`, `call_peel`, `call_peel_section`,
    `call_literal_section`, `let_type_add`, `type_sweep`, `let_template`, `sequence`, `sig_evolution`, `trait_method_rename`, `inherent_method_rename`, `item_intent`, `fn_joint`, `member_rename`, `mod_rename`, `assoc_rename`, `repeat_fold`, `partial_strip`, `single_block`, `literal_frame`, `macro_interior`, `macro_rule_body`, `macro_enclosing`, `verbatim`, `delete`, `move`)
or `"ordinary"` for wholly-ordinary sections (empty `rule`, `saved_bytes: 0`). `sites` are
the section's changed old-side lines covered by the hunk's emitted
range (anchor-only move pastes report `[]`, exactly — there is
nothing to cover, so their `sites_exact` is true). `sites_exact` is
true when the list is proven exact (the emitted range parsed and
either pre-image totality backs the coverage or the list is empty)
and false when it is a covering superset (section-visible evidence or
a missing header). `totality` is `"pre-image"` when the section had
pre-image text to count against, else `"section-visible"`; `proven`
is true exactly for `"pre-image"` (a `"section-visible"` entry is a
covering guess, never proof). `saved_bytes` is the hunk's byte win:
single-hunk firings report the exact section saving (the one hunk is
the whole win); multi-hunk section rewrites measure each hunk — the
ordinary native hunk bytes its old-range covers minus its own emitted
block bytes (saturating; `0` for anchor-only pastes) — never a split
of a section total, so those entries need not sum to the section
delta. `group` is
the WI-7 cross-file intent label: `""` when ungrouped, else
`rename_Old_to_New` shared with every hunk of the same single-delta
intent across files (rename-like detectors only — same-callee,
template, verbatim, delete, and move hunks never group; divergent
deltas never group; single-file repeats never group except
`trait_method_rename` and `inherent_method_rename`, whose def+call
intents live in one file and group under the shared name even for a
lone section). Grouped hunks share one
`rule=<group>` name in patch bytes (WI-7b): the re-emission renames
each grouped hunk from its positional `rN` to the shared
`rename_Old_to_New` and orders file sections definition-first
(item/fn-header renames before call/use/macro sites, stable otherwise;
ungrouped sections never move). Per-group T375 re-basis: the shared
name must cost strictly less than the group's `saved_bytes` sum, else
that group falls back to positional `rN` names while keeping the
`group` label. An ordinary entry carries `declined: [{detector,
reason}]` — the complete trial log for the section (WI-17): every
detector the chain attempted records exactly one decline in chain
order (deduped): pair-chain families individually
(`item_rename`, `fn_slot_sweep`, …, `literal_frame`, including the
per-hunk singles arms `precision_definition_single`,
`impl_header_single`, `attr_replace_single` for dynamic-detector
arms); composite drivers by driver name (`partition`, `partial`,
`subhunk`, `call_peel`, `call_literal`, `macro_interior`,
`macro_rule_body`, `macro_enclosing`, `attr_sweep`, `verbatim`,
`delete`, `definition_addition`, `definition_addition_follow`,
`variant_addition_follow`, `use_regroup`); section gates by gate name
(`section` with `precondition:crlf-content`, `veto:stranded-joint`,
or `veto:cross-kind-member`). Every
detector the chain never attempted for the section records one
`not-tried` entry in registry order appended after the attempts —
absence is data, not silence. `reason` is chain-level under the
closed taxonomy (WI-17 slice 1): `no-match` (shape decline, no
candidate), `guard:<kind>:sem=X:ord=Y:allow=Z` (a candidate priced
out by the byte guard — `kind` is `byte` under strict T375 with
`allow == 0`, `precision` under the WI-7 allowance; `sem` is the
losing emission bytes, `ord` the normalized ordinary span, `allow`
the precision allowance, so the overflow `sem > ord + allow` is
computable from the report alone), `totality:<found>:<want>`
(site-count mismatch — WI-17 slice 4 wires the call family first
(`call_sweep` / `call_arg_add` whole-section, per-hunk `call_sweep`
single: the shape matched and the file-wide count `found` differs
from the diff pair count `want`; unwired detectors still decline
shape-or-count as `no-match`), `veto:<name>` (named veto: off-diff
use, opaque hit — WI-17 slice 4 wires `veto:opaque-hit` for the same
call family when a macro/string twin covers a changed line —,
ambiguous anti-unification), `precondition:<name>` (named
failed shape premise, incl. the LF-only section premise); the pre-WI-17 bare `byte-guard` /
`precision-guard` spellings stay parseable but are no longer emitted
anywhere (WI-17 slice 2 unified every guard veto path), and the bare
section-gate spellings (`crlf-content`, `stranded-joint`,
`cross-kind-member`) stay parseable but are no longer emitted anywhere
either (WI-17 slice 3 unified every section-gate veto onto the closed
`veto:` / `precondition:` forms). Detector-internal guard vetoes surface as
`no-match` — reasons are never detector-inferred. Text mode prints
the same trial to stderr as one `declined: <file> <detector>
<reason>` line per entry (stdout stays the patch byte-identical).
Whole-patch pre-pass declines (moves, item/member/mod
intents, cross-file intents) are patch-global and never attributed
per-section: the pre-pass-only detectors (`move`, `item_intent`,
`item_joint_impl`, `fn_joint`, `mod_rename`) never appear in a
section's `declined` list (`member_rename`, `assoc_rename`, and
`trait_method_rename` also fire as per-section chain detectors and keep
appearing there under their chain meaning). There is deliberately no
patch-level `declined` key — a pre-pass decline is the absence of a
semantic entry, by design (WI-17 close-out: the absent pre-pass
`not-tried` listing is a final non-goal, not deferred work — a
patch-global absence cannot be attributed per-section without
inventing a section it never ran on, and the per-section absence is
already data via `not-tried`); partially-compressed sections (semantic plus ordinary
remainder in one emitted file section) carry one remainder ordinary entry —
`sites` are the original section's changed lines minus the union of the committed
semantic prov sites for the emitted rules (run-scoped, so context-carrying bodies
never over-claim remainder lines), `declined` is the section's chain-level list
(deduped; inner subhunk/verbatim classification probes stay on a throwaway by
design — the whole-hunk chain attempt already records per-family evidence).
Sections nothing tried (headers-only, non-`.rs`)
stay silent passthrough, as do pre-pass-emitted sections with no
native hunk (already semantic). `version` is `4` — unified with `applyrs` via the
single `rsmpl_patch::REPORT_VERSION` constant (WI-20; `v2`/`v3`
carried the same success keys with their own number while sharing the
error object, `v1` with a hand-built error payload).
Budget (bytes beyond `patch`):
≈100 B fixed + ≈165 B per `hunks[]` entry + ≈5 B per covered site
(`sites` arrays grow with matches — evidence, not overhead) +
≈75 B per `declined` entry (fixed detector/reason vocabulary —
evidence, not overhead).
Operational pin: overhead ≤ `150 + 190·H + 10·T + 75·D` bytes (`H` = hunks,
`T` = total sites, `D` = total `declined` entries). No relative % bound, same rationale as D.2.
The completed log needs no truncation arm (WI-17 close-out): the entry
count is bounded by construction (one per attempted detector family,
gate, and registry completion, deduped), so the `75·D` term prices
every length and the totals stay the default truth at any size.

- Fn/item rename: exactly one `-`/`+` line pair in the section,
  column-0, same kind/vis/suffix with one ident delta. Plain
  top-level `fn` (no `pub`/`async`/generics/return-type/where —
  those need explicit slots the single-rename arm does not emit)
  compresses to `- fn Old(...)` / `+ New(...)` with `# lang=rust`;
  other kinds (`struct`/`enum`/`trait`/`type`/`mod`/`union`/`const`/
  `static`, vis echoed) compress to bare `- kind Old` /
  `+ kind New` pairs. Single indented (method/block-level) and
  single generic-fn lines are excluded (no token win as a slot hunk).
- Item-intent raising (WI-15a, detector `item_intent`): exactly one
  `split_item` definition pair (`struct`/`enum`/`trait`/`type`/`const`/
  `static`/terse-`fn`; `macro_rules!` declines to its own detector and
  `mod` joins its file move via `mod_rename` below) joined with N-1 same-delta use lines (`impl Old`, `s: Old`,
  `-> Old`, `Old { … }`, …) across one or more files compresses to the
  single definition hunk with `# sweep` + `# follow=uses`. The
  cross-file scratch-apply probe is the totality (the emission applied
  to an in-memory pre-image clone must reproduce exactly the native
  plus lines at the claimed sites with every other line byte-identical;
  twin definitions, missing pre-images, and unbalanced sections
  decline). The byte-win guard compares the one hunk against the
  replaced native sections. WI-1 vis-qualified `fn` arm: a top-level `fn`
  pure-rename definition with a concrete vis from the closed set
  (empty, `pub`, `pub(crate)`, `pub(super)`, `pub(in <path>)` with
  `::`-separated ident segments) and equal qualifier/signature
  suffixes (any signature) joined with N-1 call-callee use lines of
  the old name (bare, qualified, turbofish; `let`-wrapped included)
  or `use`-tail deltas (`use crate::a::old;` → `use crate::a::new;`,
  direct group members, `as`-preserving, `pub use` re-export forms)
  emits the terse `- [vis] fn Old` / `+ [vis] fn New` diet form with
  `# sweep` + `# follow=uses` (never a `V` hole — the intent proved
  one concrete vis; `# sweep` already carries file scope at any
  spread). WI-5 name-only widening: the vis/qualifier run may carry
  token-equal `const`/`async`/`unsafe`/`extern ["ABI"]` qualifiers
  (name-only delta over the full slot vector — vis, qualifier-list,
  generics, params, return, `where` all token-equal); the emission
  carries the qualifiers concretely (`- async fn foo` /
  `+ async fn bar`, never a `Q` hole). One-line definitions (`fn foo(x: i32) -> i32 { x }`) join
  the same way when the bodies are token-identical (the suffix covers
   the body). `pub(…)` paths naming the old ident itself, signature
   evolutions, method calls, fields, bare value paths, macro heads,
   glob `use` shapes, nested member paths (`crate::{x::old}`
   suffixes), intermediate-segment changes, and second-hop re-exports
   veto the whole intent (never a partial claim; a vis/qual evolution
   alongside the rename is the joint paragraph below, never this pure
   arm); the probe doubles as the uniqueness guard (an off-diff
   same-name call or same-tail `use` fails the unchanged-leg).
  Item rename+attr joint (WI-5 slice 1, same detector): one definition
  rename pair plus exactly one attached attr-delta pair (same file,
  attr line directly above the def line) joined with at least one
  claimable use emits the follow hunk with the attr delta carried
  above the def pair (the authored Preserve-delta spelling). Double
  attr deltas, detached attr hunks, non-attr co-residents, `fn` defs
  (WI-4 B2 owns those), and attr+def without a use stay ordinary
 (a detached attr hunk falls back to the pure rename intent with the
 attr left ordinary). Variant-add, multi-addition, and cross-file-member
 joints stay ordinary (future slices).
  Item rename+vis/qual joint (WI-5 slice 1, same detector): a
  top-level `fn` definition pair differing in the name plus exactly
  one of {visibility, qualifier-list} (`pub fn foo` →
  `pub(crate) fn bar`, `fn foo` → `async fn bar`, `fn foo` →
  `extern "C" fn bar`; generics/params/return/`where` token-equal)
  joined with at least one claimable use from the pure arm's domain
  (call-callee lines, `use`-tail deltas) emits the follow hunk with
  the concrete evolved terse pair (`- pub fn foo` /
  `+ pub(crate) fn bar` — never a `V`/`Q` hole: the intent proved one
  concrete evolution, and the engine lowers rename+vis/qual
  simultaneously). Qualifiers span `const`/`async`/`unsafe`/
  `extern ["ABI"]` (WI-5-R1). Multi-line headers classify on their
  first line (WI-5-R3): same one-slot law with the post-name first-line
  tails token-equal instead of the full signature slots (the joint probe
  owns the remainder lines). Two evolving slots, unknown future
  qualifier tokens, self-referential vis (`pub(in old)`), indented
  (method) headers, `;` decls, reflowed headers (brace parity mixed),
  signature deltas on later header lines, twin definitions (counted over
  every vis/qual spelling of `fn old`, single-line or first-line),
  unbalanced sections, and missing pre-images veto the whole intent (the
  joint probe doubles as the uniqueness guard). Bare-item rename+vis joint
  (WI-5-R2, same detector; `struct`/`enum`/`trait`/`union`/`type`/
  `const`/`static`): a definition pair differing in the name plus
  exactly one closed-set vis step (`pub struct Old` →
  `pub(crate) struct New`, suffix token-equal) joined with at least
  one claimable use from the pure arm's domain emits the follow hunk
  with the concrete evolved terse pair (`- pub struct Old` /
  `+ pub(crate) struct New` — never a hole; the engine rewrites the
  vis slot alongside the name, preserving generics/bodies/attrs).
  Qualifier-bearing lines (`unsafe trait`, `static mut`) never parse
  as a bare vis prefix, so vis joints over them stay ordinary; `mod`
  joints decline (WI-15c owns mod intents); same-name vis evolution,
  vis+attribute double slots, unknown vis, self-referential vis, and
  suffix (generics/`where`/body-shape) deltas stay ordinary (multi-line
  bare headers classify on their first line when the suffix is token-equal,
  like their `fn` twins).
- Joint `impl Trait for Type` raising (WI-3, WI-3b, detector `item_joint_impl`):
  exactly one `trait` definition rename pair plus exactly one self-type
  definition rename pair (`struct`/`enum`/`union`/`type` alias with equal
  RHS) plus exactly one composed `impl` header pair (`impl OldTr for
  OldTy<sfx>` → `impl NewTr for NewTy<same sfx>`, each side either a
  single unqualified ident or a qualified path with a byte-equal prefix
  and a renamed tail; a generic `impl<…>` prefix composes when byte-equal
  across the pair, WI-12 slice 4) in one file, plus zero or more
  follow-use lines (each renaming at least one full half
  ident-boundary, no dedicated call-callee/macro-head position, no
  def-shaped third delta — the `extra_claimable` gate), compresses to
  two hunks under one shared rule (`# sweep` + `# follow=uses` each,
  trait pair first per the def-before-member law). The composed header
  and the extras are never stated: the two follows rewrite them at
  apply time. Totality is the D13 joint scratch-apply probe (composed
  emission on a `pre` clone must reproduce the plus lines at every
  claimed site with every other line of every file byte-identical —
  off-diff same-name uses fail the unchanged leg) plus twin-definition
  vetoes and the T375 byte-win guard. Cross-file groups (WI-12 slice
  1) raise the same way with one section per def file sharing the
  rule (both hunks carry the follow — the composed header needs both
  halves rewritten); single-section-pure shapes stay with the
  single-file arm byte-identically. A use-only section mixing claimed
  and unclaimed pairs declines at section level even across hunks
  (WI-12 slice 3): a file keeping an ordinary remainder is never
  Rust-parsed, so no cross-file follow can reach into it — only
  fully-collapsed (follow-owned) use-only files compose; def files
  stay pure (any unclaimed pair in a def path vetoes).
- Rename+body joint (WI-20, `let`-single widened WI-13 slice 1, two-pair widened WI-13 slice 2, three-pair widened WI-13 slice 3, slot-evolution triple widened WI-13 slice 4, detector `fn_joint`): a rename-only header (slices 1–3) or a Joint-A-shaped evolving header (slice 4: evolving ⊆ {vis, qual, params, ret} with params or ret evolving, at most two evolve, generics/where preserved) plus one to three body line pairs (each pair both sides one `syn::Expr`, or both sides one plain same-binder `let` with an init and no attributes, differing text) plus callee-only calls raises to the single follow hunk carrying the header `...` pair plus the raw body pair lines (`- x + 1` / `+ x + 2`, `- let y = 1;` / `+ let y = 2;`); the engine applies the header evolution through the shared splice and each body pair span-scoped inside the renamed body (one `body`-labelled edit per pair, hard `RSMPL_MATCH_CONFLICT` per pair on zero/multiple/out-of-body hits, never a sweep). Non-Joint-A slot evolutions (generics evolution, three slot evolutions, vis-only evolution, B1-shaped arg evolutions), attr deltas, binder-changing `let` pairs (per-pair veto), init-less `let` pairs, four-or-more body pairs, multi-line bodies, and twin in-body hits suppress as today; totality is the same scratch-apply probe plus the byte-win guard. WI-15 slice 1: a body pair the joint declines (binder-changing `let`, init-less `let`) no longer sinks a provable header+calls intent when fused in the def hunk — the def-hunk split (see the Joint-A cell) fires the header+calls as a plain follow hunk with the declined bodies as verbatim leftovers.
- Joint rename+evolution raising (WI-4a, detector `fn_joint`; two-slot
  widened WI-8 slice 1): exactly
  one column-0 `fn`-header pair with a name delta *plus* a params or
  return slot delta (a pure rename is `item_intent`'s, never joint;
  a vis-only rename stays the terse intent's too)
   joined with N-1 call-callee use lines of the old name (bare,
   qualified, turbofish, `let`-wrapped) in the same file — or across
   files (WI-4c: the def in one file with callee-only calls across
   files raises to the same one follow hunk at the def; Joint-B
   cross-file shapes stay ordinary, see below) — compresses to
  the single header-pair hunk with `# sweep` + `# follow=uses`, the
  pair carrying both deltas (`- fn foo(x: i32) { ... }` /
 `+ fn bar(x: u32) { ... }`, native spacing preserved). The vis stays
 in the WI-1 closed set and may now evolve alongside the
 params/return delta (two slots: vis+params, vis+return; the check
 normalizes `syn` pretty spacing, so `pub ( crate )` admits like
 `pub(crate)`); qual/generics/where are
 preserved and a third evolving slot (vis+params+return) vetoes;
 only the call-callee-only class fires (arguments
 token-identical — the follow pass never adds arguments, so a hunk
 claiming otherwise would fail its own probe). Totality is the same
 scratch-apply probe plus the unique-definition guard and the
 byte-win guard. WI-15 slice 1 (def-hunk split): when the def hunk
 fuses the header pair with one to three body statement pairs (each
 side a `syn::Stmt` or `syn::Expr` — binder-changing lets, init-less
 lets, plain expressions) that the body joint declines, the header
 plus call-site pairs still fire as this follow hunk while each body
 pair parks as a tight verbatim leftover hunk in the same section
 (original coordinates; the chain pass may re-harvest admittable
 bodies as singles afterwards). The split needs a Joint-A-shaped or
 rename-only header (rename-only admission is disjoint from
 `item_intent`: it requires a nonempty parked set), at most three
 parked pairs, no attr/doc/item lines in the def hunk, no
 attr-shaped unbalanced hunk in the group, and a reduced callee-only
 claim; B1-shaped headers with in-hunk bodies still suppress
 (slice-2 residue). The parked bytes leave the claimed side of the
 gate while the synthesized header overhead prices emission-side
 under the precision allowance. Claim-then-veto suppresses instead of fragmenting:
  a joint shape that fails any fire gate (non-single-expr body delta,
  attr delta, arg-delta
  calls, generics/where/qual delta, vis-only or triple-slot delta,
  twin defs, probe or byte-win fail)
  suppresses its old callee name for each involved file, so the call
  hunks stay wholly ordinary instead of emitting a rename-claiming
  sweep over a definition delta they cannot see.
- Joint-B composition raising (WI-4b, detector `fn_joint`): the two
  vetoed classes above that compose under one probe raise as two
  hunks under one shared rule with `# sweep` and no follow. B1
  (rename+param-add): the def hunk is exactly the header pair (name
  delta, params evolve, return preserved, vis closed-set and
  preserved, qual may evolve alongside params (WI-8 slice 6),
  generics/where preserved) and every call pair
  anti-unifies to one uniform arg-add template (N≥2, equal
  qualifier/turbofish, token-identical E-prefix, one shared concrete
  trailing suffix: `- foo(E0)` / `+ bar(E0, 0)`). B2 (rename+attr):
  the def hunk is exactly one header pair with a name delta plus one
  `#[…]` attr add/remove (all other header slots preserved) and every
  call pair is a uniform callee-only site (N≥2, one arity/qualifier/
  turbofish). The whole-composition scratch-apply probe is
  authoritative (for B2 a splice-aware probe: the attr insertion
  shifts lines); each fired shape also clears the byte-win guard, and
  N=1 joints suppress (the two-hunk emission never beats the byte-win
  guard at N=1 — pinned; pre-pass emissions skip per-section
  re-processing (WI-4d), so N≥2 survival is structural and the
  unique-single detector never sees the call-template hunk). The
  composed report is `partial` with the def hunk's orphans naming
  exactly the sibling call sites (sibling-covered by construction —
  the agent's next action is the sibling hunk under the same rule).
   Out of scope (stays ordinary): Joint-B multi-file groups (a
   cross-file rename+arg-add/attr shape suppresses to ordinary via the
   involved-path set — the call-template sweep is file-scoped, so no
   two-section emission; never a fragment — WI-4c), return-plus-
  arg-add doubles, slot-plus-attr doubles, and hetero call shapes.
- Mod-intent raising (WI-15c, detector `mod_rename`): exactly one
  `mod old;` → `mod new;` declaration pair (inline `mod old {` bodies
  decline — there is no file to join) joined with N-1 same-delta
  `old::` path-use lines (`use crate::old::…`, bare `old::…` uses and
  expression paths) across one or more files compresses to the single
  definition hunk with `# sweep` + `# follow=uses`. An accompanying
  headers-only `rename from/to` section (flat `old.rs`→`new.rs` or
  `old/mod.rs`→`new/mod.rs` layouts, dirs otherwise equal) is preserved
  verbatim as the native file-move prelude — it carries no pairs and
  never counts toward the byte-win guard. Totality is the cross-file
  scratch-apply probe (def proven plus ≥1 use proven, every other line
  byte-identical); twin `mod` definitions, missing pre-images,
  unbalanced sections, basename-mismatched or multiple renames, and
  mixed proven/unproven sections decline. File/section-pure partial:
  proven files collapse to the one hunk, unproven files stay ordinary
  (call-callee and macro-head uses belong to their own detectors and
  veto the join; the per-section `use_prefix` chain may still compress
  the unclaimed remainder, each hunk independently totality-checked).
- Field/variant member rename (WI-9a authored, WI-9b raised single-file, WI-15b raised cross-file; WI-14 anchored follow): a `struct S {` / `enum E {` context header plus a member pair (`- old: T` / `+ new: T` with equal types; bare `- Old` / `+ New` with equal suffixes). The plus-instantiation postcondition carries the same span-local posture as item renames (the new ident must appear at the reported spans). Raising (detector `member_rename`): exactly one definition pair plus N-1 use-site lines sharing the ident delta compress to the definition hunk *with* `# follow=uses`; the scratch-apply line-equality probe is the totality (the authored emission applied to the pre-image must reproduce exactly the native plus lines at the changed sites with every other line byte-identical). The probe runs the WI-14 anchored follow, so sibling same-named definitions no longer veto raising (unproven uses fail the probe instead); bare-`Old` without a glob anchor, qself paths, and macro interiors fail the probe and stay ordinary; twin definitions stay ordinary. Single-section shapes raise per section; multi-section / cross-file shapes raise via the whole-patch `member_rename` pre-pass (one def pair joined with same-delta use lines across files, file/section-pure partial: proven files collapse to the one hunk, unproven files stay ordinary, mixed hunks veto the intent (no partial-hunk claim — a def hunk carrying an unclaimable pair declines the whole intent), mixed sections partial (proven hunks collapse, leftover hunks stay verbatim in the same section)). Def-only singles (WI-19b, N = 1 definition pair, no use sites): the definition hunk alone with no `# follow=uses` (a follow would rewrite uses the diff did not touch) in the concrete directive-free spelling, under the precision guard; same container recovery + scratch probe.
- Associated-item rename (WI-13 slice 1 authored, WI-27 follow, WI-28 raised single-file, WI-12 raised cross-file): K ≥ 1 `type`/`const` definition pairs in `trait`/`impl` blocks (one per block, same kind and ident delta) plus use-site lines sharing the delta through a `::Old` tail (`Self::Old`, `S::Old`, `Tr::Old`, `<S as Tr>::Old`). WI-3 emits the single document-first def hunk with `# follow=uses` (WI-1 claims the sibling impl definitions, so one hunk suffices); the scratch-apply line-equality probe is the totality, falling back to one hunk per enclosing block (verbatim headers, shared rule, covering-block ranges, follow on the first) when the single-hunk probe fails. Bare-`Old` spellings, two-trait same-name classes, hetero kinds, and multi-def blocks decline the whole-section follow to ordinary; cross-file intents raise via the whole-patch `assoc_rename` pre-pass (WI-12: K ≥ 1 def pairs across files, one section per def file with one hunk per block sharing the rule, `# follow=uses` on the document-first hunk only; the workspace scratch-apply probe is the totality — an off-diff same-name `::Old` use anywhere vetoes; single-section pure shapes stay with the per-section detector). Def-only singles (WI-19b, N = 1 definition pair, no use sites, no follow, concrete directive-free spelling, precision guard) still compress per hunk with the opaque remainder left ordinary verbatim in the same section (disjoint ranges) — the WI-28 whole-section veto owns the follow intent, not the follow-free def hunk. Detector name `assoc_rename`.
- Pair partition: N ≥ 2 `-`/`+` pairs in a whole balanced section whose
  pair chain declined (heterogeneous deltas share no template) but
  every pair is a single-pair item rename above (including the plain
  `fn` arm, whose gates apply per pair) or a single-delta `impl`
  header rename below (WI-19c: self-ty `impl Old` → `impl New` with no
  `for`, or trait-side `impl OldTr for SameTy` → `impl NewTr for
  SameTy` with the self-ty byte-equal; a byte-equal `impl<…>` generic
  prefix and byte-equal-prefix qualified trait tails compose like the
  bare header; the double-delta line, both
  sides changed, is never a single). Nested `struct`/`enum`/`union`/
  `mod` items (mod-nested, block-level) classify like top-level ones
  (WI-19c: leading whitespace tolerated; the bare spelling is
  indent-insensitive in the engine); indented `fn` lines are methods
  (slot sweep owns them) and indented `type`/`const`/`static`/
  `macro_rules!` lines stay with their own detectors. All-or-nothing:
  one unclaimable pair vetoes the whole section (a mixed hunk stays
  ordinary whole, never half-semantic), as do duplicate minus lines
  (identical pairs belong to the verbatim sweep). Emission is one file
  section with one hunk per pair (per-pair covering ranges, positional
  `rN` rules; distinct deltas never group; nested-item ranges may nest
  inside their enclosing-item ranges — edits are disjoint spans, so
  nesting is sound under Section 10.6). The section-level byte-win
  guard applies (the combined section must beat the ordinary section);
  a losing partition falls back to the per-hunk path. Detector name
  `item_rename`.
- Partial partition (WI-19): the all-or-nothing gate above is kept
  per native hunk, not per section. When ≥ 1 hunk is fully claimable
  (every pair in it a single-pair item rename or WI-19c `impl` single
  under the same duplicate/totality gates) and ≥ 1 hunk is not, the
  section emits the claimed hunks as semantic per-pair hunks and keeps
  the rest ordinary verbatim in the same file section (hunk ranges are
  disjoint, so mixing is sound under Section 10.6). A single mixed
  native hunk splits per the WI-4 sub-hunk law below; what remains a
  non-goal is splitting a single pair across two semantics, regrouping
  non-adjacent same-class pairs, and splitting container-joint shapes.
   Guard is the scaled precision guard (semantic ≤ ordinary
   + 16 × claimed pairs). Detector name `item_rename`.
 - Sub-hunk partition (WI-4): a single balanced native hunk with N ≥ 2
   pairs whose whole-hunk detectors decline splits at detector-class
   boundaries. Maximal contiguous runs of one single-pair classification
   (the detector the shared chain picks for the pair alone, proven
   without reserving rules) are each offered to the shared chain —
   whole multi first, then per-pair singles, all-or-nothing per run —
   while runs that decline stay ordinary verbatim in place (document
   order preserved, never reordered; hunk ranges are disjoint, so mixing
   is sound under Section 10.6). An all-unproven run may still split
   into two firing multi halves (bounded split-search, both halves ≥ 2
   pairs, no recursion). An interleaved key (A B A) vetoes the whole
   hunk (order is never manufactured); duplicate minus lines veto
   (verbatim territory); duplicate emitted minus patterns veto (terse
   shapes would overlap-match); any suppressed-callee mention vetoes
   (WI-4a suppression posture); a `struct`/`enum` rename beside a member
   line of that same container vetoes, proven by the carrier
   exactly-one-container law (WI-5/WI-21 joint territory is never
   split). Totality is the D13 composed scratch-apply (the emission
   applied to a `pre` clone must reproduce the native post tree
   byte-identically) plus the scaled precision guard (semantic ≤
   ordinary + 16 × claimed pairs). `None` pre-image stays ordinary.
   Detector names are the firing groups' own (provenance one entry per
   semantic hunk); diffrs fixture `129-subhunk-partition-fires`.
   Stranded-use veto (WI-5 slice 2a): when a native hunk holds both an
   evolution pair (any `definition_evolution` shape) and the rename's
   use while the definition sits in another hunk, the whole section
   stays ordinary — the bare definition single would otherwise claim
   the rename while an ordinary hunk rewrites the stranded use (the
   item analogue of the WI-4 claim-then-veto suppression). Pure
   definition + pure evolution hunks (no stranded use) still partition;
   `fn` definitions never trigger this veto.
    Struct rename + field-type joint (WI-5 slice 2b, first): a def hunk
    holding the `struct` rename plus exactly one attached field-type
    evolution pair (same file/section, field's pre-image carrier is the
    renamed struct) plus claimable uses (in any file — WI-12 slice 5:
    the follow is workspace-wide and the D13 probe counts every touched
    file; the member itself stays same file/section by the container
    law) raises as one follow hunk with
    the member pair below the def pair, proved by the one scratch-apply
    probe (D13) under strict T375. Two evolutions, another container's
    evolution, field-type lines beside an enum rename (cross-kind),
    variant lines beside a struct rename (cross-kind), and alias/const
    shapes never joint; multi-addition insertions never joint (field or
    variant); a member in another file than its container cannot joint
    (container law non-goal). Field-addition joints raise per
    2b-next2b-first, variant-addition joints per 2b-next2b-next.
    Struct rename + field-deletion joint (WI-21 slices 1 + 3): a def
    hunk holding the `struct` rename plus exactly one attached
    field-deletion line (same file/section — either a distant
    pure-deletion hunk or a hunk adjacent to the def change; the
    holder's pre-image carrier is the renamed struct) plus claimable
    uses (in any file — WI-12 slice 2: the follow is workspace-wide and
    the D13 probe counts every touched file, so a touched file leaving
    an `Old` use unrewritten vetoes the joint) raises as a shared-rule
    two-hunk emission (the field-deletion
    hunk naming the old container, then the terse rename follow hunk),
    proved by the one multi-file full-post probe (D13) under strict
    T375; diffrs fixture `142-struct-rename-field-delete-fires`. Uses
    of the deleted field the native leaves untouched surface as
    `off-target`/`shadowed`/`second-hop`/`macro-hidden` orphans of the
    deletion hunk (capping at `partial`); uses the native
     deletes/rewrites fail the probe and the joint declines. Two
     deletions and attr+deletion combos never joint-fire (slice 5,
     probed): distant deletions partition — the rename/attr joint
     still fires alone with the deletions riding one ordinary
     remainder hunk (stranded lines in remainder sites) — while
     adjacent multi-deletion hunks stay wholly ordinary via the
     shape-B trap. Cross-kind members (WI-21 slice 4): a field-shaped
    deletion under an `enum` rename, or a variant-shaped deletion under a
    `struct` rename, vetoes the joint AND suppresses the bare rename single —
    the section stays wholly ordinary with the stranded delta named in declined
    provenance (`section` / `cross-kind-member`, sites carry the deletion line).
    Enum rename + variant-deletion joint (WI-21 slices 2 + 3): a def
    hunk holding the `enum` rename plus exactly one attached
    variant-deletion line (same file/section — either a distant
    pure-deletion hunk or a hunk adjacent to the def change; the
    holder's pre-image carrier is the renamed enum) plus claimable uses
    (in any file — WI-12 slice 2: same workspace follow and D13 probe
    law as the struct half; a touched file leaving an `Old` use
    unrewritten vetoes the joint) raises as a shared-rule two-hunk
    emission (the variant-deletion
    hunk naming the old container, then the terse rename follow hunk),
    proved by the one multi-file full-post probe (D13) under strict
    T375; diffrs fixture `143-enum-rename-variant-delete-fires`. Same
    orphan law as the struct half (a rewritten `Old::B` → `New::B`
    pair rides as a claimable rename use; the dangling `New::B`
     orphans). Two deletions and attr+deletion combos never
     joint-fire: same partition law as the struct half (slice 5;
     cross-kind suppression above).
    Enum rename + variant-evolution joint (WI-5 slice 2b-next, first):
    the enum half of the same law — a def hunk holding the `enum`
    rename plus exactly one attached variant-evolution pair (tuple
    payload with equal arity and ≥1 differing type, struct payload with
    equal field names and ≥1 differing type, or discriminant with
    differing exprs; same file/section, variant's pre-image carrier is
    the renamed enum) plus claimable uses (in any file — WI-12 slice 5:
    workspace-wide follow like the struct half; the member itself stays
    same file/section by the container law) raises as one follow hunk
    with the member pair below the def pair, proved by the one
    scratch-apply probe (D13) under strict T375. At most one joint fires
    per def hunk (attr, field, and variant joints are mutually
    exclusive). Bare variant renames, field-type lines, another
    container's evolution, two evolutions, and alias/const shapes never
    joint; multi-addition insertions never joint (field or variant);
    a member in another file than its container cannot joint (container
    law non-goal). Single-addition joints raise per 2b-next2b-first
    (field) and 2b-next2b-next (variant).
- Slot sweep: N ≥ 2 `-`/`+` pairs, each a single-line
  `{`-terminated fn header (`syn`-validated as `ItemFn`/`ImplItemFn`)
  differing in exactly the fn ident (per-site vis/qual/generics/
  params/return/where byte-equal, heterogeneous across sites, one
  shared `Old`→`New` delta). Uniformly column-0 sites emit the
  free-fn sweep (`- V Q fn Old G (...) W {` /
  `+ V Q fn New G (...) W {`); uniformly indented sites emit one
  hunk per enclosing `impl` block with the block's verbatim header
  line from the pre-image (Section 13.4: the engine filter matches
  only the named block), all sharing one positional rule (`rN`)
  within the section (cross-file intents share `rename_Old_to_New`
  after WI-7 grouping); each hunk clears its own engine probe
  (twin headers veto to
  ordinary). Mixed indent or divergent deltas stay
   passthrough. The slot sweep is the zero-delta (rename-only) special
   case of the signature-evolution sweep below; both detectors run, slot
   first, and rename-only classes never reach evolution.
   Rename-only header singles (WI-19b, N = 1): a lone
   vis-prefixed / generic / `->` / `where` header pair the terse
   item-rename declines compresses to the same sweep spelling under
   the precision guard. Same `slot_holds` totality; a pre-image is
   required (the wide wildcard minus has no section-visible
   evidence without an occurrence count).
  One-line (`}`-terminated, brace-balanced by whole-item parse,
  rename-only with byte-identical body) lone method definitions in
  `impl` blocks — and lone default-body definitions in `trait` blocks
  (WI-6 slice 2, same spelling with the verbatim trait header as the
  filter) — compress instead to the compact filtered single
  (verbatim filter header plus one inline-body
  `V Q fn Old G (...) W { ... }` pair, no frame, no directives)
  under the same guard with the four-guard filter-block budget; the
  file-wide count degrades to the opaque-only veto there (the filter
  names the block), proved by a scratch-apply of the exact emission.
- Signature-evolution sweep: N ≥ 2 `-`/`+` pairs, each a single-line
  `{`-terminated fn header, where the name plus any of
  vis/qualifiers/generics/params/return/where carry one uniform delta
  and bodies stay untouched. Every non-name slot is either preserved
  (per-site minus == plus) or evolving (one uniform plus text; the
  minus base may be hetero). Uniform bases emit concrete
  (absent → omitted); hetero preserved bases emit wildcards
  (`V`/`Q`/`G`/`W`, params `(...)`, return omitted = `Any`); hetero
  evolving bases wildcard the minus side while the plus side sets the
  slot wholesale; removals emit minus-concrete/plus-omitted. Two
  engine-spelling laws are enforced at emission: `V`/`Q` only ever
  appear as the pair (a lone `V`/`Q` is inference-ambiguous and
  concrete+wildcard mixes are rejected), and a concrete plus-side
  return drops the `where` wildcard (`-> T W` is unparsable — the
   minus side keeps `W` and widens the return to omission instead).
   Divergent plus sides and hetero-base removals stay ordinary (the
   engine's slots are all-or-nothing: `...`, concrete, or omitted).
   Pre-image totality is the generic engine-count check: the emitted
   minus header must match exactly N fn items in the pre-image
   (macro/string twins never count — the engine never enters them);
   indented classes emit one verbatim-header hunk per enclosing `impl`
   block, each probed separately. Detector name `sig_evolution`.
   The signature-evolution detector owns shared-name classes (one
   uniform name delta plus slot deltas); hetero-name slot-only
   classes belong to the header-evolution sweep below — the two are
   disjoint by construction (uniform names vs hetero preserved names).
     Vis/qual/return evolution singles (WI-19b, N = 1, WI-14 slice 3):
     a lone vis/qual-evolving header pair compresses as a precision
     single (uniform-by-construction) under default `one` (no
     `# allow=many`: exactly-one-set proof via the engine-count probe).
     Empty preserved long slots stay concrete (`()`/omission, never
     `G (...) W` — the diet fix for short headers); non-empty preserved
     longs keep the wildcard (concrete pretty-printing would re-render
     types like `&str`/`Option<String>` and fail the byte-compare) with
     the `-> T W` law unchanged (a preserved return keeps the minus
     omission plus `W`). Single-line `}`-terminated items parse via
     their header prefix (text up to the first `{`); the scaffold
     emission rewrites the header only (engine preserves the body
     verbatim, including single-line bodies). A lone
     params/generics/where evolution stays ordinary — the sig
     rewrite would re-render the type with pretty spacing (`Box<[i32]>`
     to `Box <[ i32 ]>`, 117 precedent), so those shapes belong to the
     type-sweep family (strict N ≥ 2) instead — except a lone
     single-type-delta line, which raises as a WI-2 bare-type literal
     single (`- Vec<i32>` / `+ Box<[i32]>`, exactly-one-type-subterm
     proof) rather than through the sig rewrite. A vis evolution joined
     with a formatting-stable return evolution (WI-2 slice 3: both
     return texts single-token, e.g. `-> i32` → `-> i64`, 113 shape —
     composite returns decline and ride the bare-type single instead),
     and a lone formatting-stable return evolution, raise as precision
     singles likewise (the evolving return rides concrete on both
     sides; omitted-return twins veto via the engine-count totality).
     Tree-canonical equality unifies the authored bare-minus spelling
     (`- fn helper(...)`) with the inferred concrete-minus spelling
     (`- fn helper(...) -> i32`) under the D17 single-site + re-prove
     discipline (WI-1s1 return-slot arm): wide-range bare templates
     proving two same-name sites stay divergent. An N = 1 sig single
     requires a pre-image (same wide-minus doctrine as the slot sweep).
     Method (indented) N = 1 vis/qual/return singles emit the canonical
    verbatim-header block hunk and pay a 64-byte method-run price (four
    guards) for the mandatory WI-4a filter block (a proof anchor, D10,
    not bloat): the whole-section arm allows 32 over its
    prelude-carrying section, the per-hunk fallback allows 64 over the
    bare pair hunk (measured ~41: filter + rule tag), and the subhunk
    scaled guard prices three extra guards per method run beyond its
    base (extra header + filter + tag, ~53 measured). Each budget prices
    the verbatim filter header per emission: headers longer than the
    measured baseline (`impl Svc {`) contribute exactly their surplus,
    so a long enclosing block never vetoes a proved single on bytes
    alone. Totality is the
    engine-count check plus the scratch-apply byte-compare in every
    path, so mixed-section method singles fire beside ordinary or
    other-class pairs; in-scope twins, params/generics/where-involving
    two-slots, and trait-context vis deltas still decline.
    Bare-item N = 1 vis singles (`struct S;` → `pub struct S;`,
    same kind/name/suffix, closed-set vis step; `fn` rides the sig sweep,
    `mod` stays ordinary, `impl`/`macro_rules!` take no visibility) raise
    as the concrete directive-free terse pair (`- struct S` /
    `+ pub struct S`, bare like renames — the engine preserves
    generics/body/RHS) under default `one` and the 16-byte precision
    guard; totality is the scratch-apply byte-compare probe, so the
    hunk claims only its covered site. Bare-item N ≥ 2 vis sweeps (one
    uniform closed-set vis step over same-kind items with per-site
    preserved names and suffixes; divergent plus sides, two-slot pairs,
    mixed kinds, and duplicate minus lines decline) raise as a single
    N-holed terse pair (`- struct N` / `+ pub struct N`) under
    `# allow=many`; the engine binds each name per site and replays it
    while rewriting the vis slot, and totality is the same scratch-apply
    probe at N sites (an in-scope twin vetoes to singles). Same-hole
    visibility evolution is allowed in authored patches (`- struct N` /
    `+ pub struct N`); differing holes stay a pattern error (rename+vis
    joints with holes are concrete-only).
- Header-evolution sweep: N ≥ 2 `-`/`+` pairs, each a single-line
   `{`-terminated fn header, where every name is preserved per site
   (`minus name == plus name`) but names differ across sites (≥ 2
   distinct), and exactly one non-name slot
   (vis/qualifiers/generics/params/return/where) carries one uniform
   delta. Slot classification and emission ride the
   signature-evolution laws above (concrete uniform bases,
   wildcarded hetero bases, the `V Q` pairing rule, the
   `-> T W` fixup). The header sweep keeps N ≥ 2 (every N = 1
   name-preserving evolution is the sig sweep's concrete-name
   class below); the emitted name is the WI-5 name hole
   `N` (`- fn N(...) {` / `+ pub fn N(...) {`). Zero evolving slots
   is nothing to raise; two or more stay ordinary (never
   half-claimed under one rule); any name delta stays with the
   rename/evolution siblings. Trait-context `vis` deltas decline at
   emission (`pub` is illegal in traits). Pre-image totality is the
   same engine-count check over the `N`-holed minus header (free fns)
   or per-`impl`-block probes (methods) — an untouched same-shape
   header vetoes to ordinary. Detector name `header_evolution`.
- Attribute-delta sweep: N ≥ 1 pure-attr hunks (bare `+#[...]`
  insertions, bare `-#[...]` deletions, or balanced attr-line
  replacements) sharing one uniform attr-line delta over hetero items.
  Owners come from the pre-image (`syn` walk: the innermost item
  containing the changed line, or the line after an insertion point);
  `None` pre-image, method/`impl` owners, mixed attr+code hunks, and
  multi-line attr spellings decline. Emission is one hunk per class in
  the WI-10a authored spelling with header tiers for fns (bare
  `- fn N(...) {`, then `- fn N(...) W {`, then the full
  `- V Q fn N G (...) W {` wildcard set) and bare pairs for items
  (`- struct N`); hetero names ride `N`, uniform names stay concrete.
  Pre-image totality is the scratch-apply byte-compare probe: the
  emitted hunk must apply to the pre-image with `matches == class
  size` and reproduce byte-identically the result of applying the
  claimed ordinary hunks (count equality alone can coincide wrongly).
    Per-class T375 (each body beats its covered hunks); N = 1 fires
    under the precision posture (`body <= ordinary + 16` — the
    definition-shape guard, WI-19b doctrine: the concrete owner header
    anchors the single site and the probe demands `matches == 1`, so
    proof replaces bytes; Fn-family singles stay ordinary on bytes).
    The whole-balanced single-pair replace rides the chain whole-block
    single-pair arm (same guard over the full section, prelude
    included); N = 1 emits the ranged owner-span diet (`@@ -s,len @@`
    with no directive — default `one` over the `syn` owner span, which
    always contains both the changed attr and the owner header), never
    the sweep dummy (whose 8 bytes kept the live short-hunk section 1
    byte over the guard); the sweep-based probe (`matches == 1`) still
    proves it, conservatively vetoing same-name twins.
   A losing class declines alone, never vetoing a winning one.
   Detector name `attr_sweep`.
- Definition-value evolution (WI-12 slice 1): one balanced pair
  (N == 1) — or N ≥ 2 uniform pairs sharing one shape class,
  identity, and exact RHS delta — compresses to the WI-11 authored
  spelling (bare `- type A = RHS;` / `- const/static … = EXPR;`
  pairs; member pairs under a recovered `struct S {` / `enum E {`
  container header). Generics-bearing aliases, vis/mut/kind/type
  mismatches, hetero deltas, and hetero containers (member N ≥ 2)
  decline; `None` pre-image declines (containers are unprovable
  without it). Totality is the scratch-apply byte-compare probe:
  the emitted hunk must apply with `matches == n` and reproduce
  byte-identically the native post lines at the changed sites.
  Guard is the precision guard (semantic ≤ ordinary + 16 bytes);
  sweep shapes keep strict T375. Field/variant single-insertion
  addition is `definition_addition` (below); construction
  follow-through is WI-12c (stays ordinary here).
  Detector name `definition_evolution`.
- Definition-member addition (WI-12b slice 1): one pure-insertion
  hunk (`-N,0` with a single `+ member` line) inside a proven
  container compresses to the WI-12b authored spelling (`struct S {`
  / `enum E {` context plus the one `+` member line, no minus
  member). `+ b: Ty` must parse as a field side (`name: Ty`,
  `syn::Type`-validated); `+ C` / `+ C(..)` / `+ C { .. }` /
  `+ C = ..` must parse as a variant (`syn::Variant`-validated,
  bare unit names accepted). The container is recovered from the
  pre-image (the `struct`/`enum` spanning the insertion point and
  lacking the member); `None` pre-image, outside-container points,
  duplicate members, two-plus lines, balanced pairs, and hetero
  containers decline. Totality is the scratch-apply byte-compare
  probe: the emitted hunk must apply with `matches == 1` and
  reproduce byte-identically the native post (insertion after line
  N, everything else untouched). Guard is the precision guard
  (semantic ≤ ordinary + 16 bytes). Unverified insertions report
  `reparse-only` (no structural plus re-match for insertions); verified
  pure insertions and member additions report `postcondition`
  (Section 24.6).
  Construction follow-through is the next bullet (WI-12c slice 1).
  Detector name `definition_addition`.
- Definition-addition struct-literal follow-through (WI-12c slice 1):
  a file section holding exactly one pure-insertion *field* addition
  (the bullet above) plus K ≥ 1 balanced single-pair hunks whose only
  delta is adding the new field to a struct literal of the same struct
  (`S { a: 1 }` → `S { a: 1, b: true }`, bare tail-expr or `let`-bound
  with a token-equal pattern, semi-ness equal) compresses to the
  definition hunk plus K per-site concrete ranged construction hunks
  (`@@ -L,1 @@` citing each changed line). Hetero added values stay
  per-site concrete (a plus-only hole would be `unbound` under
  Section 18.6, and ranges pin each value to its line so values cannot
  cross-apply). Either side carrying `..` rest, a renamed path, a
  changed sibling field, a second extra field, a variant insertion,
  multi-line pairs, or any unrelated hunk declines the whole group
  (all-or-nothing over the section's hunks — the per-hunk fallback
  then still compresses the definition insertion alone; `..`-rest
  sites with no diff are untouched by construction of the compare).
  Totality is the joint scratch-apply byte-compare probe: the emitted
  hunks must apply with one match each AND reproduce byte-identically
  the native post. Guard is the precision guard scaled per emitted
  hunk (semantic ≤ ordinary + 16 × hunks). Variant/match-arm
  follow-through is the next bullet. Detector names `definition_addition` (def
  hunk) and `definition_addition_follow` (construction hunks).
- Definition-addition variant/match-arm follow-through (WI-12d):
  a file section holding exactly one pure-insertion *variant* addition
  (the bullet above) plus K ≥ 1 pure-insertion arm hunks (`-M,0` with
  one `+ <arm>,` line) whose arms name the added variant compresses to
  the definition hunk plus K per-site concrete ranged arm hunks
  (`@@ -M,1 @@` + `  match <scrut> {` + `+ <arm>,`). Each arm's path
  must name the variant: a qualified prefix must equal the definition
  enum (`E::C`), or a bare `C` needs a `use …E::*` glob line in the
  pre-image (unanchored bare arms decline the group). Hetero bodies
  ride per-site concrete. Any balanced pair, second insertion, variant
  mismatch, unanchored bare, or unrelated hunk declines the whole
  group (all-or-nothing — the per-hunk fallback still compresses the
  definition insertion alone). Totality is the joint scratch-apply
  byte-compare probe and the guard is the scaled precision guard
  (`≤ ordinary + 16 × hunks`). Emission always appends before the
  closing brace (native arms sorted into the middle decline via the
  probe); cross-file arms stay ordinary. Detector names
  `definition_addition` (def hunk) and `definition_addition_follow`
  (arm hunks).
- Trait-method rename sweep: N ≥ 2 `-`/`+` pairs, each a single-line
  fn header (`;`-terminated trait declarations or `{`-terminated
  definitions and trait defaults, `syn`-validated; `}`-terminated
  single-line definitions such as `fn run(&self) {}` classify on the
  same `{` emission, which reproduces them byte-identically) differing in
  exactly the fn ident (per-site vis/qual/generics/params/return/
  where byte-equal, heterogeneous across sites, one shared
  `Old`→`New` delta). Every changed line must sit in a `trait` block
  or an `impl` block (pre-image totality), with at least one site on
  each side: pure trait-side classes stay ordinary (no impl side
  shares the intent) and pure impl-side classes stay with the slot
  sweep. WI-3 emits the single document-first trait hunk with
  `# follow=uses` (the engine claims trait-proven `impl <Trait> for
  <Self>` definitions as `rename` spans; inherent `impl S` defs never
  claim); the scratch-apply line-equality probe is the totality (the
  single hunk applied to the pre-image must reproduce every native
  plus line). WI-23 admits anchored call lines sharing the ident delta
  (cheap ident-boundary gate, scratch over defs+calls authoritative;
  unproven/shadowed calls fail the scratch and stay ordinary, never a
  def-only partial; WI-24 pins `S::old`/`Self::old`/`<S as T>::old` tails
  on the same gated path — proven tails ride the follow, unrelated `X::old`
   tails fail the scratch and stay ordinary). Cross-file intents raise via the whole-patch `trait_method_rename` pre-pass (WI-12: trait decl + trait-`impl` defs + anchored calls across files, same one-trait/two-side/uniform-delta gates; emission is the single trait-block hunk with `# follow=uses` — the follow owns every remote def and call, so sibling def hunks would double-claim and are never emitted; a failed single-hunk probe declines to ordinary, never a def-only partial; single-section pure shapes stay with the per-section detector). Probe failure falls back to one hunk per enclosing
  block with verbatim headers sharing the `rename_Old_to_New` rule
  (no follow, def-only classes; each hunk clears its own engine probe,
  twin headers and untouched twins veto to ordinary). WI-22: any def in an
  inherent `impl S` block declines the whole class to ordinary (different
  items, never the K-hunk fallback). Divergent per-pair
  WI-22b: trait tails must match (last segment, `crate::T` proves `T`) or the whole class declines to ordinary (different items, `T::old` vs `U::old`).
   WI-22c: cross-file grouping joins on the method `(old,new)` delta only; each grouped intent stays per-file same-trait (WI-22b), so the shared rule is token sharing, not a global same-trait claim.
   suffixes and mixed indent stay passthrough. Detector name `trait_method_rename`.
 - Inherent-method rename (WI-6): the trait detector's inherent sibling.
   Every def pair sits in an inherent `impl` block of one self type
   (last-segment discipline, D8) with one uniform ident delta, plus at
   least one anchored call line sharing the delta (def-only classes stay
   with the slot sweep; trait-block and `impl <T> for _` defs veto).
    Emits the single document-first impl-block hunk with `# follow=uses`
    iff the block probe and the full-set scratch probe reproduce every
    changed line; otherwise ordinary (unproven/shadowed calls fail the
    scratch, never a def-only partial). Cross-file intents raise via
    the whole-patch `inherent_method_rename` pre-pass (WI-12: inherent
    defs of one self type + anchored calls across files, same
    one-self-type/uniform-delta gates; emission is the single
    document-first impl-block hunk with `# follow=uses` — the follow
    owns every remote same-self-type def and anchored call, so sibling
    def hunks would double-claim and are never emitted; a failed
    single-hunk probe declines to ordinary, never a def-only partial;
    single-section pure shapes stay with the per-section detector).
    Detector name
    `inherent_method_rename` (Precision under the WI-7 allowance — the
    block probe + full-set scratch prove the site set, so the small
    case fires as one follow hunk).
- Call sweep: N ≥ 2 `-`/`+` pairs, each a bare single-segment call
  (`syn`-parsed, trailing `;` tolerated) with per-site textually
  identical args, uniform arity, and one shared callee delta →
  one `# allow=many` hunk (`# expr E0…` per arity position;
  arity 0 needs no metavars). A leading-uppercase concrete callee
  (tuple-struct and enum-variant constructors) emits backticked
  (`` `Old`(E0) `` → `` `New`(E0) ``, SPEC 15.4): bare `Old` would
  otherwise infer as an `expr` hole (SPEC 15.8), failing loud on the
   plus side when the callee changes and over-matching when it does
   not. The same rule covers the trailing-arg-addition,
   trailing-literal, and head-peel sweeps below, the qualified-call and
   turbofish sweeps (backtick on the last segment only —
   `` crate::`Old`(E0) ``, `` `Old`::<u8>(E0) ``; a full-path quote is
   an invalid literal token), and the method sweeps (backtick on the
   method ident — `` Receiver.`Old`(E0) ``); turbofish-suffixed method
   idents stay plain (the `::<` suffix defeats hole inference, so the
   pattern is already literal), as do macro names (never inferred) and
   `use` paths (backticks rejected there — the concrete path is
   already literal). A bare pattern never matches a leading-`::` site
   (parity, probed); a bare single-segment `::foo` rename stays
   ordinary (no prefix to anchor the win).
     Precision singles (WI-7 slices 1-23, `PrecisionUnique`): N == 1
  whole-line bare-call callee-only pairs (slice-1, `call_holds` over
  `old_callee/arity`), whole-line method-to-method renames (slice-2,
  `method_holds` over `old_method/arity`), whole-line
  method-to-function refactors (slice-3, `method_holds` over
  `old_method/arity` with a receiver-prepend delta), whole-line
  general-expression pairs (slice-4) and `let`-init general-expression
  pairs (slice-5, same WI-5 init entry as the multi sweep), each via
  `template_pair_holds` at n=1 over the single-pair anti-unification:
  exactly one hole plus an operator/member/type anchor delta;
  zero-hole and two-delta shapes veto, each with a pre-image proving
  exactly one site file-wide with zero opaque hits. Slice-6 adds
  bare-call arg-template wrap pairs (whole-line plus `let`-init via the
  shared T449 entry, same callee, exactly one structural wrap from the
  multi's `wrapper_tpl` vocabulary — method-receiver and reference
  shapes — all other args holes; zero/two wraps, pure concrete deltas,
  and permuted holes veto) via `template_pair_holds` at n=1 over the
   emitted call pair. Slice-7 adds method-arg-template wrap pairs
   (whole-line plus `let`-init via the shared T449 `parse_method_line`
   entry, same receiver and same method, exactly one structural wrap
   from the same `wrapper_tpl` vocabulary via the shared
   single-pair core — all other args holes; receiver/method deltas,
   zero/two wraps, pure concrete deltas, and permuted holes veto) via
   `template_pair_holds` at n=1 over the emitted `Receiver.m(...)`
   pair. Slice-8 adds bare-call and binary inner-wrap pairs
   (`use_it(E0)` → `use_it(f(E0))`, `use_it(E0)` → `use_it(E0 + 1)`)
   for both outer entries through the same shared single-pair core:
   the N ≥ 2 column-uniformity gates are vacuous-but-blocking at N=1,
   so these two vocabularies prove structurally from one pair
   (`wrapper_tpl_single`: exactly one arg/side token-equals the minus,
   every other position a concrete anchor); zero holes, two holes,
   and pure concrete deltas veto as before. Slice-9 adds method
   one-arg-hole inner-wrap pairs (`use_it(E0)` → `use_it(r.foo(E0))`,
   plus the `Receiver.` outer twin) for both outer entries through the
   same shared core: the method one-arg-hole column gate is likewise
   vacuous-but-blocking at N=1, so the shape proves structurally
   (`wrapper_tpl_single` method arm: receiver-is-hole or exactly one
   arg token-equals the minus, every other position a concrete anchor
   — receiver/token is the anchor beyond the hole); zero holes, two
    holes (`r.foo(x, x)`, `x.foo(x)`), and pure concrete deltas veto.
    Slice-10 adds whole-line bare-call arg-permutation pairs
    (`swap(E0, E1)` → `swap(E1, E0)`, whole-line plus `let`-init via the
    shared T449 entry) in a sibling module (`perm_single.rs`, D5 split):
    the N ≥ 2 perm sweep owns sub-line-only classes and vetoes
    whole-line pairs, so the single proves structurally from one pair
    (same callee, uniform arity k ≥ 2, plus args a non-trivial
    permutation of the minus args — pure token multiset bijection via
    first-unused mapping; identity, non-bijective deltas, callee
    deltas, and macro/unsupported args veto; the callee token is the
    anchor beyond the holes). The
    callee/method/function/operator token is the anchor beyond the holes
    (`Receiver`/`E`; uppercase rides backticks, same rule as above).
    Emission is the ranged allow-one spelling (tight original range,
    directive-free, default `allow=one`); the `let`-init emission stays
    the bare anchored template (pattern/type ride each site's own
    wrapper) and carries the same T361 byte-compare pre-reservation as
    slice-4. Guard is the precision guard (semantic ≤ ordinary + 16).
    Slice-11 adds whole-line method-call arg-permutation pairs
    (`Receiver.foo(E0, E1)` → `Receiver.foo(E1, E0)`, whole-line plus
    `let`-init via the shared `parse_method_line` entry) as the twin of
    the slice-10 entry: same receiver and same method (receiver deltas
    stay with the m2f single, method deltas with the slice-2 single),
    uniform arity k ≥ 2, plus args a non-trivial token multiset
    bijection of the minus args; identity, non-bijective deltas,
    receiver/method deltas, and macro/unsupported args veto. TOOL-5
    hygiene (same slice): the slice-6/7/11 defensive `syn::Expr`
    re-parse runs on the raw callee/method (backticks are RSmPL-level
    literal spelling, never Rust syntax); the emission keeps the
    backticked literal. `None` pre-image vetoes (no guessing); an
    untouched same-shape twin vetoes (same-arity for call/method
    shapes, same-template for general, same-trailing-literal for
    literal); non-matching twins never count. Slice-12 adds
    whole-line bare-call trailing-literal pairs
    (`make_widget(E0, E1, false)` → `make_widget(E0, E1, "100644")`,
    whole-line plus `let`-init via the shared T449 entry): same
    callee, uniform arity k+1 with k ≥ 1 (zero-hole pure-concrete
    pairs veto — verbatim-shaped, general-expr single precedent;
    fixture 57 `foo(&x)` → `foo(x)` stays ordinary), leading k args
    token-equal minus-vs-plus, trailing old/new concrete literals with
    a real delta; callee deltas, leading-arg deltas, identity, and
    macro/unsupported args veto; uniqueness proved by
    `call_literal_holds` at n=1 keyed on the trailing old literal.
    Slice-13 adds whole-line method same-literal pairs
    (`Receiver.configure(alpha_argument_1, first_value_foo, false)` →
    `Receiver.configure(alpha_argument_1, first_value_foo, "100644")`,
    whole-line plus `let`-init via the shared `parse_method_line`
    entry): same method, same per-site receiver, uniform arity k ≥ 1,
    old/new arg token-tuples with a real delta; method deltas, receiver
    deltas, identity, and macro/unsupported args veto; uniqueness proved
    by `method_literal_holds` at n=1 keyed on the exact old-args tuple.
    The emission is the concrete tuple (no `E` arg holes — a holed
    template would match sites the tuple-keyed proof never counted;
    `Receiver` is the only hole). Slice-14 adds sub-line bare-call
    callee-only pairs (`foo(alpha_argument) + 1` →
    `bar(alpha_argument) + 1`, bare-statement and `let`-init framings
    via the shared `side_expr` entry): the pair must not parse
    whole-line (slice-1 owns that shape), `extract_bare_sites` yields
    exactly one site with a real callee delta and the frame rule holds;
    multi-subterm lines decline (multi territory by site count);
    uniqueness proved by `template_expr_holds` at n=1 plus the T361
    byte-compare. Slice-15 adds sub-line method-to-method rename-only
    pairs (`val.old_method(alpha_argument) + 1` →
    `val.new_method(alpha_argument) + 1`, bare-statement and `let`-init
    framings via the shared `side_expr` entry): the pair must not parse
    whole-line (slice-2 owns that shape), `extract_method_sites` yields
    exactly one site with a real method delta and the frame rule holds;
    multi-subterm lines decline (multi territory by site count);
    uniqueness proved by `template_expr_holds` at n=1 over the emitted
    `Receiver.old(E0…)` template plus the T361 byte-compare; uppercase
    idents ride backticks with the raw-method re-parse check (TOOL-5).
    Slice-16 adds sub-line method-to-function pairs
    (`val.old_method(alpha_argument) + 1` →
    `new_func(val, alpha_argument) + 1`, bare-statement and `let`-init
    framings via the shared `side_expr` entry): the pair must not parse
    whole-line (slice-3 owns that shape), `extract_method_fn_sites`
    yields exactly one site with a receiver-prepend delta and the frame
    rule holds; multi-subterm lines decline (multi territory by site
    count); uniqueness proved by `template_expr_holds` at n=1 over the
    emitted `Receiver.old(E0…)` template plus the T361 byte-compare;
    uppercase idents ride backticks with the raw-probe re-parse check
    on both sides (TOOL-5). Slices 17-23 land the remaining sub-line
    families (general, literal, perm, wrap -- see below). Slice-17
    adds sub-line bare perm pairs (`swap(alpha_arg, beta_arg) + 1` ->
    `swap(beta_arg, alpha_arg) + 1`, bare-statement and `let`-init
    framings via the shared `side_expr` entry): the pair must not parse
    whole-line (slice-10 owns that shape, including the T449 `let`-init
    entry), `extract_arg_permutation_sites` yields exactly one site with
    a real non-trivial permutation and the frame rule holds;
    multi-subterm lines decline (multi territory by site count);
    uniqueness proved by `template_pair_holds` at n=1 over the emitted
    pair plus the T361 byte-compare; uppercase callees ride backticks
    with the raw-callee re-parse check (TOOL-5). Slice-18 adds sub-line
    method perm pairs (`svc.configure(alpha_arg, beta_arg) + 1` ->
    `svc.configure(beta_arg, alpha_arg) + 1`, bare-statement and
    `let`-init framings via the shared `side_expr` entry): the pair
    must not parse whole-line on both sides (slice-11 owns that shape,
    including the T449 `let`-init entry),
    `extract_method_perm_sites` yields exactly one site with a real
    non-trivial permutation and the frame rule holds; receiver deltas
    stay with the m2f single and method deltas with the slice-15
    single (both veto in the extractor); multi-subterm lines decline
    (multi territory by site count); uniqueness proved by
    `template_pair_holds` at n=1 over the emitted
    `Receiver.m(E0, E1)` / `Receiver.m(E1, E0)` pair plus the T361
    byte-compare; uppercase methods ride backticks with the raw-method
    re-parse check (TOOL-5). Slice-19 adds sub-line general-expression
    pairs (`foo(alpha_arg + 1);` -> `foo(alpha_arg + 2);`,
    bare-statement and `let`-init framings via the shared `side_expr`
    entry, detector in the `general_subline.rs` sibling per D5): the
    pair must not anti-unify whole-line (slices 4–5 own that shape, via
    both the whole-line and the `let`-init entries),
    `extract_general_sites` yields exactly one inner general site with
    a real single-hole delta and the frame rule holds; unchanged
    subterms (a nested base such as `self.alpha`) are skipped, never
    vetoed — twin-counting is the engine totality over the emitted
    pair; multi-subterm lines decline (multi territory by site count);
    uniqueness proved by `template_pair_holds` at n=1 over the emitted
    pair plus the T361 byte-compare. Slice-20 adds sub-line bare-call
    trailing-literal pairs (`foo(bar(alpha_arg, 1));` ->
    `foo(bar(alpha_arg, 2));`, bare-statement and `let`-init framings
    via the shared `side_expr` entry, detector inline in `literal.rs`
    — the file sits well under the D5 gates): the pair must not form a
    whole-line literal pair with `prefix >= 1` (slice-12 owns that
    shape; the emission tail and the `call_literal_holds` counter are
    shared, so no bypass around its twin veto via a second counter —
    wrapper pairs with `prefix == 0` fall through, since the whole-line
    single vetoes them as zero-hole),
    `extract_literal_sites` yields exactly one inner site with a real
    trailing-literal delta (`k >= 1`; unchanged pairs and zero-hole
    pairs are skipped, never vetoed — twin-counting is the engine
    totality over the emitted triple) and the frame rule holds;
    multi-subterm lines decline (multi territory by site count);
    uniqueness proved by `call_literal_holds` at n=1 over the emitted
    triple plus the T361 byte-compare; uppercase callees ride backticks
    with the raw-callee re-parse check (TOOL-5). Slice-21 adds sub-line
    method same-literal pairs (`foo(svc.configure(alpha_arg, 1));` ->
    `foo(svc.configure(alpha_arg, 2));`, bare-statement and `let`-init
    framings via the shared `side_expr` entry, detector in the
    `method_subline.rs` sibling per D5 — `methods.rs` sat at ~1962, so
    the detector plus the shared emission tail cannot land inline
    without breaking the 2000 gate; the whole-line slice-13 arm keeps
    its shape checks and delegates to the same tail, net-shrinking
    `methods.rs`): the pair must not form a whole-line method
    same-literal pair (slice-13 owns that shape; the emission tail and
    the `method_literal_holds` counter are shared, so no bypass around
    its twin veto via a second counter),
    `extract_method_literal_sites` yields exactly one inner site with a
    real arg-tuple delta (unchanged pairs are skipped, never vetoed —
    twin-counting is the engine totality over the emitted exact
    old-args tuple; method/receiver/arity deltas veto) and the frame
    rule holds; multi-subterm lines decline (multi territory by site
    count); uniqueness proved by `method_literal_holds` at n=1 over the
    emitted tuple plus the T361 byte-compare; uppercase methods ride
    backticks with the raw-method re-parse check (TOOL-5). Slice-22 adds sub-line
    method-arg-template wrap pairs (`foo(svc.process(alpha_arg));` ->
    `foo(svc.process(alpha_arg.clone()));`, bare-statement and
    `let`-init framings via the shared `side_expr` entry, detector in
    the `method_subline.rs` sibling per D5 — `methods.rs` sits at ~1933
    and `arg_template.rs` at ~1875, so the detector cannot land inline
    in either without breaking the 2000 gate; the shared single-pair
    `antiunify_call_args_single` core is reused, not mirrored): the pair
    must not form a whole-line method wrap pair (slice-7 owns that shape
    via the same parser plus the shared core, so there is no bypass
    around its twin veto via a second counter),
    `extract_method_wrap_sites` yields exactly one inner site whose args
    anti-unify to exactly one wrap plus holes. The extractor collects
    OUTERMOST method calls only — a wrapper that is itself a method call
    (`a.clone()`) adds a plus-side subterm, so flat pairing would veto
    every such wrap; turbofish/non-ident calls never record but still
    gate their interiors. Unchanged pairs are skipped, never vetoed
    (slice-19/20/21 discipline — twin-counting is the engine totality
    over the emitted holed pair, which keys on method + arity, so any
    same-method same-arity twin vetoes even with different args —
    slice-7 precedent); pure-concrete deltas veto the line (literal
    territory — slices 20/21 own them), permuted holes veto the line
    (perm territory), method/receiver/arity deltas veto the line
    (slice-15/m2f territory); multi-subterm lines decline (multi
    territory by site count); uniqueness proved by
    `template_pair_holds` at n=1 over the emitted holed pair plus the
    T361 byte-compare; uppercase methods ride backticks with the
    raw-method re-parse check (TOOL-5). Both wrap arms sit BEFORE the
    literal arms: a holed wrap beats the concrete literal spelling
    where both prove the same site set (reference/binary wrappers
    upgrade from the slice-20/21 concrete spelling); the general arm
    above declines wraps by construction (`Call`/`MethodCall` subterms
    are never general sites). Slice-23 adds sub-line bare-arg-template
    wrap pairs (`foo(bar(alpha_arg));` ->
    `foo(bar(alpha_arg.clone()));` and the same-level twin
    `foo(alpha_arg) + 1` -> `foo(bar(alpha_arg)) + 1`, bare-statement
    and `let`-init framings via the shared `side_expr` entry, detector
    inline in `calls.rs` -- the sub-line bare-call owner, well under
    the D5 gates): the pair must not form a whole-line bare wrap pair
    (slice-6 owns that shape via the same parser plus the shared core,
    so no bypass around its twin veto via a second counter),
    `extract_bare_wrap_sites` yields exactly one site via a recursive
    structural worker: same-callee pairs whose args anti-unify to
    exactly one wrap plus holes site at their own level; pairs the
    core declines descend into their args pairwise (deep-inner wraps
    add a plus-side bare level, so flat pairing would veto them on
    counts); binary and method-call frames are transparent, with any
    frame delta left behind vetoing via the frame rule (never
    partially claimed); pure-concrete deltas veto the line (literal
    territory), permuted holes veto the line (perm territory),
    callee/arity deltas veto the line (slice-14 territory); unchanged
    pairs are skipped, never vetoed (twin-counting is the engine
    totality over the emitted holed pair, which keys on callee +
    arity, so any same-callee same-arity twin vetoes even with
    different args -- slice-6 precedent); multi-subterm lines decline
    (multi territory by site count); uniqueness proved by
    `template_pair_holds` at n=1 over the emitted holed pair plus the
    T361 byte-compare; uppercase callees ride backticks with the
    raw-callee re-parse check (TOOL-5). The sub-line wrap family is
    closed (whole-line slices 6-9 plus sub-line slices 22-23).
- Same-callee trailing-arg-addition sweep (T447): N ≥ 2 `-`/`+`
  pairs, each a bare single-segment call with one shared callee,
  uniform minus arity `k`, and uniform plus arity `k + t` (`t` ≥ 1)
  whose leading args token-equal per site (bound as `# expr E0…`)
  while the trailing plus-only args are token-equal concrete text
  across all sites (emitted verbatim — per-site trailing metavars
  would bind nothing, T442). Pre-image totality reuses the call
  minus-arity count (hetero-trailing siblings veto).
- Same-callee trailing-literal-delta sweep (T448): N ≥ 2 `-`/`+`
  pairs, each a bare single-segment call with one shared callee and
  uniform arity `k + 1` whose leading `k` args token-equal per site
  (bound as `# expr E0…`, hetero across sites) while the trailing arg
  is token-equal concrete text across all sites on each side with a
  real old/new delta (emitted verbatim, T442-safe). All-identical
  classes stay for the tighter verbatim sweep (peel precedent, T422).
  Pre-image totality keys on callee + trailing old literal
  (`call_literal_holds`, T420-style): same-arity sites with a
  different trailing literal never match and never veto. Call sites
  inside `let` inits unwrap to their init call (the whole line parses
  as a `Stmt`, declined as `Expr::Let`; the emission stays a bare-call
  pattern and the engine rewrites the call sub-expression in place).
  A trailing literal hiding a macro invocation anywhere inside stays
  ordinary (WI-9b2-delim): TokenStream-normalized macro text
  (`wrap(old_mac ! (1))`) is unmatchable by the engine, so the sweep
  declines and those pairs fall through to the nested macro sweep (or
  ordinary). Boolean `!`/`!=` never contain a macro node and keep firing.
  Cross-hunk classes compress non-whole sections (T449: singleton
  `-U0` hunks split by distance or pure additions elsewhere, e.g.
   spaced `let` sites — peel-driver precedent); all-identical classes
   stay for verbatim and the emitted bodies are invisible to it. Hunk
   atomicity (WI-9e, same index-based fixed point as the macro-interior
   sweep): a class fires only if every balanced hunk holding a member
   pair is fully covered by passing classes — a class whose member hunk
   also holds an uncovered pair drops to a fixed point (cascade:
   dropping uncovers hunks) and the section falls through to ordinary;
   rules reserve only for survivors.
 - Hetero call-arg-template sweep (WI-5): N ≥ 2 `-`/`+` pairs, each a
   bare single-segment call with uniform arity `k` ≥ 1 (turbofish and
   multi-segment callees stay with their detectors) and a callee that
   is token-equal across all sites or one shared delta (`O` → `N`,
   uniform on both sides), where every argument position independently
   is (a) token-equal concrete on both sides (uniform positions replay
   the first site's normalized spelling, hetero preserved positions
   ride `Ej`), (b) a whole-arg hole (`Ej` on both sides, ordered value
   correspondence per Section 15.7), or (c) a uniform wrapper — one
   `Ej` occurrence inside a plus template with a literal anchor
   (method-call `Ej.clone()`, bare-call `f(Ej)`, binary `Ej + 1` /
   `1 + Ej`, shared reference `&Ej`; every other plus shape vetoes).
   Plus templates may permute holes (`f(E0, E1)` → `f(E1, E0)`);
   dropped or duplicated holes veto (a duplicated hole would need
   repeat-equality the emission cannot state); at least one position
   carries a real delta (no-delta classes are the callee-only sweep's,
   never this detector's). An argument hiding a macro invocation
   anywhere inside vetoes (TokenStream-normalized macro text is
   unmatchable, WI-9b2-delim precedent); all-identical classes stay
   for the tighter verbatim sweep (T422 precedent). One `# allow=many`
   hunk (tight original range, Section D.5) with diet holes
   (`- old(E0)` / `+ new(E0.clone())`, no declarations, Section 15.8;
   a leading-uppercase callee rides backticks, Section 15.4).
   Pre-image totality is the real-pair engine-count check
   (`template_pair_holds`): the emitted pair must match exactly N
   sites in the pre-image — an untouched twin vetoes. Detector name
   `call_arg_template`. Sub-line whole-arg permutation (WI-6 slice 2)
   is the same family at subterm granularity: N ≥ 1 pairs whose every
   call subterm is a same-callee same-arity bare call with one uniform
   non-trivial arg permutation across all subterms (multiset-equal
   args, ordered correspondence per Section 15.7; macro args veto;
   the frame check parks each minus subterm on a placeholder so a swap
   cannot alias), emitting `swap(E0, E1)` → `swap(E1, E0)` with total
   subterm sites ≥ 2 (a single site stays ordinary, T375). Pure
   whole-line classes decline here and fire as the whole-line sweep
   above.
 - Hetero method-arg-template sweep (WI-5): the method-call sibling —
   N ≥ 2 `-`/`+` pairs, each `recv.m(args)` with per-site token-equal
   receivers (hetero across sites, bound as `Receiver`; a per-site
   receiver delta is the method-to-function domain, never guessed
   here) and the same method/arity/hole discipline as the bare-call
   sibling (method token-equal or one shared delta; no turbofish, no
   `&`-led sides). One `# allow=many` hunk (tight original range):
   `- Receiver.m(E0)` / `+ Receiver.m(E0.clone())` (uppercase method
   backticked, Section 15.4). Totality is the same real-pair check.
   Detector name `method_arg_template`.
  - General-expression sweep (WI-6): N ≥ 2 `-`/`+` pairs, each parsing
    whole-line as one `syn::Expr` of the same variant (binary, unary,
    cast, field, index, reference, await — `Call`/`MethodCall`/macro
    whole-line shapes stay with their tighter detectors; macro or
    unsupported subterms veto), anti-unified to one template pair with
    at most one `E{i}` hole. Hole positions are subterms token-equal
    within each pair but hetero across sites (ordered value
    correspondence, Section 15.7); every other position is a literal
    anchor token-equal across all sites on its side; at least one
    anchor carries the delta (`E0 + 1` → `E0 + 2`, `E0.timeout` →
    `E0.timeout2`, `&E0` → `&mut E0`). No-delta and multi-hole
    classes stay ordinary. One `# allow=many` hunk (tight original
    range, Section D.5) with diet holes, no declarations. Pre-image
    totality is the real-pair engine-count check
    (`template_pair_holds`): the emitted pair must match exactly N
    subterms in the pre-image — an untouched twin vetoes. `let`-init
    subterm fallback (WI-5): when no pair parses whole-line but every
    pair parses as `let PAT[: TY] = INIT;` with init present and no
    `let-else` diverge, the init column anti-unifies through the same
    core (same ≤1-hole, macro/unsupported vetoes); the template must
    be anchored (neither side a bare literal) and prove N through the
    same totality check, and emission is the bare anchored template
    (the engine rewrites it inside each `let`, preserving pat/ty).
    Whole-line classes keep their winner — the fallback only fires
    when whole-line declines uniformly. Detector name `general_expr`.
    Preference (WI-5/D10): where an anchored general template and a
    bare literal prove the same site set, the anchored form wins —
    a bare-literal emission is legal only when the literal is the
    whole matched unit, never when extracted as a subterm of a larger
    expression an anchored template proves.
  - Control-position sweep (WI-8b): N ≥ 2 `-`/`+` pairs whose changed
   lines each carry the changed subterm in one uniform single-line
   control position — match-arm guard (`_ if GUARD => …`), arm body
   (`PAT => BODY`), closure body (bare `|x| BODY` or a `let` whose
   init is a closure), `if`/`while` condition, `for` iterable (pattern
   token-equal per pair), single-expression `let`-`else` diverge
   (head token-equal per pair), or `await`/`?` base (whole-line or
   `let`-init spelling, same wrapper both sides) — with all N pairs
   extracting through the same position kind (mixed positions
   decline). The subterm columns anti-unify through the WI-6
    general-expression core (same ≤1-hole discipline, same macro/
    unsupported vetoes); call-shaped subterm columns route through
    the call-arg-template core instead (bare `Call` with a
    per-side-uniform single-segment callee, `MethodCall` with a
    per-pair-preserved receiver emitted as `Receiver.m(...)`;
    qualified/turbofish callees stay ordinary). Emission is the bare
    WI-6 template pair (`- E0 > 0` / `+ E0 >= 0`) in one `# allow=many`
    hunk (tight original range, Section D.5) with diet holes, no
    declarations (field bases infer `expr`, WI-23). Totality is the
    same whole-file real-pair check:
   an untouched twin anywhere — including the same subterm in a
   plain body expression — counts N+1 and vetoes to ordinary instead
   of rewriting the body (position-scoped counting and control
   frames in emission stay future work). Detector name
   `control_template`.
   - Literal-frame sweep (WI-7): N ≥ 2 `-`/`+` pairs, each parsing as
   one `syn::Stmt` (expression statements, `let`s, `return`s, `const`/
   `static` items via `Stmt::Item`; header/attr/control lines fail to
   parse and stay ordinary) and differing from its partner in exactly
   one literal token — string, char, number, or bool, same kind on both
   sides (byte/C/raw-string spellings veto; macro-body deltas contribute
   no literals and veto) — with a uniform `(old, new)` pair across
   sites while the surrounding frames stay hetero. One `# allow=many`
   hunk (tight original range, Section D.5) with the bare literal pair
   (`- "old"` / `+ "new"`), which the engine matches as literal
   subterms wherever they sit. Guards: all-identical line classes stay
   for the tighter verbatim sweep (T422 precedent); the old literal
   occurs exactly once per minus line (one template application must
   reproduce the plus line); single-site classes stay ordinary.
   Extracted-subterm veto (WI-5/D10): when every pair parses as a
   `let` with init and the init column anti-unifies to an anchored
   general template proving N, the bare pair declines so `general_expr`
   claims it. Pre-image
   totality is the same real-pair engine-count check
   (`template_pair_holds`): macro interiors and string-in-string never
   count (the engine never enters them), an untouched twin vetoes.
   Detector name `literal_frame`. Non-`let` statements (e.g.
   `return x + 1;`) stay bare in this slice (named residue for a
   `return`/general subterm follow-up).
 - Array-head-peel call-migration sweep (T441 slice 1): N ≥ 2 `-`/`+`
  pairs, each a bare single-segment call `f(D, &[H, M, T…])` →
  `g(D, M, &[T…])` — the minus 2-arg array-literal second arg sheds
  its shared string-literal head `H` while the tail replays
  positionally into a new trailing slice (statement-level lines: `let`
  inits and bare call-statements alike — the engine rewrites the call
  sub-expression in place). One `# allow=many` hunk per shared
  `(f, g, H, tail-length)` class (`# expr D` + `# expr M` +
  `# expr T0…` per tail position; all-identical classes stay for the
  tighter verbatim sweep). Pre-image totality keys on callee + exact
  array length with the unescaped head (other arities/lengths never
  match and never count). Cross-hunk classes compress non-whole
   sections (pure additions elsewhere veto whole-section balance but
   not this sweep); the emitted single-pattern bodies are invisible to
   the later verbatim pass. Hunk atomicity (WI-9e, same index-based
  fixed point as the macro-interior sweep): a class fires only if
  every balanced hunk holding a member pair is fully covered by passing
  classes — a class whose member hunk also holds an uncovered pair
  drops to a fixed point (cascade: dropping uncovers hunks) and the
  section falls through to ordinary; rules reserve only for survivors.
- Let type-addition sweep (WI-4a, first anti-unification shape): N ≥ 2
  `-`/`+` pairs, each a complete single-line `let PAT = INIT;` →
  `let PAT: T = INIT;` — the pattern and initializer token-equal per
  site and hetero across sites (bound as inferred `P0`/`E0`, no
  declarations, SPEC 15.8), with one uniform concrete type `T` across
  all sites (a varying type would need an unbound plus-side `T0`,
  T442-illegal, so it stays ordinary). Per-site pattern/init deltas,
  `let-else`, macro/closure/unsupported inits, and all-identical
  classes (verbatim stays the tighter winner, T422 precedent) stay
  ordinary. Pre-image totality counts every untyped with-init
  no-diverge `let` (`let_untyped_holds`): the minus template matches
  exactly that domain (ascription agreement is strict, so typed twins
  never match and never count), and the count must equal N.
  Framed class (WI-14 slice 4 + arm/branch residue): N ≥ 2 pairs
  whose every line unwraps through one `Stmt` rung — single-statement
  one-line `fn` frames (`fn f0() { let x = E; }` →
  `fn f0() { let x: T = E; }`), match-arm blocks
  (`1 => { let a = E; a },` → `1 => { let a: T = E; a },`, exactly one
  `let` per block), or `if`/`else` branches
  (`if c { let a = E; a } else { … }`, `} else { let a = E; a }`,
  exactly one `let` across then/else; `else if` chains stay ordinary)
  — raise to the identical bare hunk (no frame text emitted, D14) when
  every pair unwraps through the same rung to an inner `let` of the
  own-line class with a frame-equal outer (a frame delta alongside the
  inner rewrite stays ordinary — never a partial claim). Each rung's
  outer (fn/arm/branch) is never itself a `Stmt::Local`, so the bare
  emission cannot match an outer `let` first. `let`-outer frames
  (`let g = || { let x = E; };`, `let r = if … { let … };`; two-`let`
  blocks as ambiguous inners) stay ordinary by construction
  (unwrapping them would hand the engine a template matching the
  outer `let`, over-matching); mixed-rung and mixed whole-line +
  framed sections stay ordinary (all-or-nothing per side). Same
  totality (the AST count descends into `fn` bodies, match arms, and
  block inits; needles are the whole minus lines).
- Type rewrite sweep (WI-4b, second anti-unification shape): N ≥ 2
  `-`/`+` pairs, each a complete single-line `let PAT: T = INIT;` →
  `let PAT: T′ = INIT;` — the pattern and initializer token-equal per
  site (a per-site init/pattern delta is a let-statement template, not
  a type template), while the ascribed types anti-unify across sites
  to one template pair with shared inferred `T0…` holes (e.g.
  `- Vec < T0 >` / `+ Box < [T0] >`, no declarations, SPEC 15.8;
  all-identical classes stay for the tighter verbatim sweep, T422
  precedent).   Guards: same `syn::Type` variant at every generalized
  node (the root constructor is the literal anchor); holes only in
  `Type` positions at depth ≤ 2 (deeper variation coarsens to the
  shallowest varying level) — except parenthesized `Fn` inputs/outputs,
  which are granted one extra level (WI-6d: the `dyn`/`impl` wrapper
  plus bound plus path triple-count a single bound position, so
  `Box<dyn Fn(..)>` holes land at depth 3); lifetimes, const-generic args,
  mutability and all other non-`Type` slots must agree across sites (a
  varying lifetime would mis-rewrite — the engine wildcards pattern
  lifetimes while the plus side would emit the first site's spelling);
  hole correspondence is pairwise (an unbound plus-side hole is
  T442-illegal; dropped or reordered holes stay ordinary). Only
  `let`-ascription pairs qualify (bare-type full lines never occur in
  `-U0` diffs of valid Rust, so no bare-`Type` path exists).
  Pre-image totality is the generic engine-count check: the emitted
  minus template must match exactly N type subterms in the pre-image
  (macro/string twins never count — the engine never enters them;
  nested matches count outer-only, exactly like the real sweep
  rewrites). Detector name `type_sweep`.
- Joint let-template sweep (WI-5 slice 1, third anti-unification
  shape): N ≥ 2 `-`/`+` pairs, each a complete single-line
  typed `let PAT: T = INIT;` → `let PAT: T = INIT′;` — the pattern and
  ascribed type token-equal per site (a per-site pat/ty delta belongs
  to another detector, never guessed here) and hetero across sites
  (bound as inferred `P0`/`T0`, no declarations, SPEC 15.8 — WI-3:
  hetero destructuring patterns anti-unify structurally, e.g.
  `Some(v)` / `Some(w)` → `Some(P0)`, depth ≤ 2 with ≥ 1 literal
  anchor and shared holes for identical per-site value vectors per
  SPEC 15.7; uniform concrete positions replay the first site's
  spelling so the real-apply stays byte-identical), while the
  initializer carries one uniform delta: either a uniform concrete
  rewrite or one `E0` hole with a uniform structural wrapper on the
  plus side (method call with varying receiver, e.g.
  `- let P0: T0 = E0;` / `+ let P0: T0 = E0.wrapping_add(1);`; bare
  call with one varying argument; binary with one varying side — each
  with ≥ 1 literal anchor and exactly one `E0` occurrence, ordered
  value correspondence per SPEC 15.7). Guards: ascriptions on both
  sides (minus-untyped classes stay with the type-addition sweep;
  per-site-preserved-init classes stay with the type sweep — disjoint
  by construction); no attributes, no `let-else`, no unbound/dropped
  holes (T442 posture); all-identical classes stay for the tighter
  verbatim sweep (T422 precedent). Pre-image totality is the real-pair
   engine-count check (`template_pair_holds`): the emitted minus must
   match exactly N typed lets in the pre-image — an untouched twin
   vetoes. Detector name `let_template` (same family). At N = 1 a
   method-wrap single fires as the allow-one holed template
   (`- let P0: T0 = E0;` / `+ let P0: T0 = E0.wrapping_add(1);`) under
   the precision guard (uniqueness = `template_pair_holds` at n = 1
   plus the T361 byte-compare; the preserved pattern hoists only a
   bare ident to `P0` or a single-argument tuple struct over a bare
   ident to `Path(P0)` — never a bare `P0`, which would also match
   unrelated lets; nested/multi-arg/`mut` patterns and bare-call or
   binary plus-shapes stay with the literal floor).
 - Untyped joint let-template sweep (WI-8 slice 1): the joint template
  without ascriptions — N ≥ 2 untyped→untyped `let PAT = INIT;` →
  `let PAT = INIT′;` pairs with per-site preserved patterns plus one
  uniform initializer delta (uniform concrete rewrite, e.g.
  `let v = const { 1 + 2 };` → `let v = const { 1 + 3 };` and
  `let v = _;` → `let v = 0;` — `const` blocks and `_` are structural
  inits, matched by the engine and admitted by the shape gate; or one
  `E0` hole with a uniform structural wrapper, e.g.
  `- let Some(P0) = E0;` / `+ let Some(P0) = E0.unwrap_or(0);`).
  Guards: no ascription on either side (ascribed classes stay with the
  joint typed template above — disjoint by construction); no
  attributes, no `let-else`; macro/unsupported inits veto
  (nested-macro classes belong to the macro-interior sweep — the
  engine never binds inside macro bodies); all-identical classes stay
  for verbatim (T422). Totality is the same real-pair engine-count
   check. Detector name `let_template` (same family). At N = 1 the
   same method-wrap gate fires allow-one (`- let Some(P0) = E0;` /
   `+ let Some(P0) = E0.unwrap_or(0);`) under the precision guard
   with the same totality and byte-compare; the `Some(P0)` anchor is
   what keeps the exactly-one-hit proof biting.
 - Tuple-position swap sweep (WI-8 slice 1): N ≥ 2 untyped→untyped
  `let (P, Q) = E;` → `let (Q, P) = E;` pairs — each minus/plus
  pattern an arity-2 tuple with the same two elements in swapped order
  (real swap per site, no `let (x, x)` self-swap), the initializer
  token-equal per site (a per-site init delta is an untyped-let
  template, never guessed here). Cross-site hetero elements ride
  positional holes (`P0` minus-first, `P1` minus-second, replayed
  swapped; uniform elements stay concrete in first-site spelling);
  hetero inits ride `E0`, uniform inits stay concrete. Macro/
  unsupported inits veto, same posture as above. Totality is the same
  real-pair engine-count check. The typed twin
   (`let (P, Q): T = E;`) stays ordinary. Detector name `let_template`
   (same family). At N = 1 the swap fires allow-one
   (`- let (P0, P1) = E0;` / `+ let (P1, P0) = E0;`) under the
   precision guard with the same totality and byte-compare; a per-site
   init delta is still an untyped-wrap shape, never guessed here.
- Statement-sequence sweep (WI-5c, fourth anti-unification shape): N
  flat `-`/`+` pairs splitting block-major into ≥ 2 blocks × K = 2–4
  `let` statements (each block's changed old lines consecutive — the
  detector only fuses runs the engine matches as windows, Section
  16.4.2), each statement position anti-unified across blocks with the
  shared let cores in order (type-addition, joint typed template,
  untyped-init template, uniform-concrete anchor for repeated identical
  edits); hetero positions ride positionally-numbered holes (`P0`/`E0`
  in position 0, `P1`/`T1`/`E1` in position 1, … — windows bind
  independently, Section 15.7), identical ones stay concrete. Guards:
  K tried largest-first (4, 3, 2); all-identical whole classes stay
  for the tighter verbatim sweep (T422 precedent); non-`let` members,
  attribute/diverge carriers, and divergent positions stay ordinary.
  A single block (N = 1) declines — single-block runs are the
  `single_block` detector below, never this one. One `# allow=many`
  hunk with K minus/plus lines (tight original range, Section D.5).
  Pre-image totality is the real-pair engine-count check
  (`template_sequence_holds`): the emitted K-line minus must match
  exactly N block windows in the pre-image — an untouched twin window
  vetoes. Detector name `sequence`.
- Single-block sweep (WI-5d, fifth anti-unification shape): one
  contiguous run of K = 2–4 hetero plain-`let` pairs (a single block
  window, N = 1; changed old lines consecutive — the engine only
  matches consecutive statement windows), each position
  anti-unified *within* its own pair: a subterm preserved verbatim
  between that pair's minus and plus rides a positional hole
  (`Pj`/`Tj`/`Ej`, same numbering as the sequence detector so window
  statements bind independently, Section 15.7), anything that differs
  stays concrete in original site spelling (span-sliced, never
  token-normalized; concrete inits ride the same engine-matchability
  gate as holes, so macro/opaque inits decline instead of emitting a
  dead hunk). No hole is shared across lines. Territory gate: the
  run must mix init families (expression kinds) — a family-uniform
  run is an earlier detector's class and its verdict (fire or veto)
  stands, never undercut by a narrower re-spelling here. Guards:
  mixed runs decline (a header, expression, or any non-plain-`let`
  member — permanent non-goal for the window fuser itself (WI-15
  slice 1 splits a fused `fn`-header-plus-statement def hunk one level
  up, in the fn-joint pre-pass: one to three `syn::Stmt`/`syn::Expr`
  body pairs park as verbatim leftovers while a Joint-A-shaped or
  rename-only header still fires its follow; more than three bodies,
  attr/doc/item lines, attr-shaped company, interleaved order, and
  B1-shaped headers stay unsplit by construction. Full-history probe:
  0/2473 paired hunks over 467 commits hold a splittable hetero let
  window); one-sided ascriptions decline
  (a type-add/removal inside the run is another detector's class);
  K = 1 true singles stay ordinary (Section D.4 N = 1 rule); K > 4
  exceeds the `Sequence` engine arm's exact-K bound; all-identical
  whole classes stay for the tighter verbatim sweep (T422 precedent).
  One allow-one hunk (tight original range, no `# allow=many`, no
  `# sweep` — a twin fails loud as too-many at apply time). Pre-image
  totality is the same real-pair engine-count check
  (`template_sequence_holds` with n = 1): the K-line minus must match
  exactly one window — an untouched twin vetoes. Detector name
  `single_block`. A narrow K = 2 hetero window (one pure type-add
  plus one typed method-wrap, WI-7 slice 2) fires as one allow-one
  window intent under the same totality and byte-compare; any other
  delta combination is not a window and falls through to the
  partition literals — the one-sided decline above stands.
- Repeat-fold sweep: N ≥ 2 `-`/`+` pairs where every minus is the same
  1-arg callee (bare `foo` or qualified `Box::new` — leading `::`,
  qself, and generic args decline here; narrower than the qualified
  sweep, whose engine spelling now covers both) applied one or more times around a plus side that is exactly the
  full-strip fixpoint (peeling that callee repeatedly from the minus
  reaches the plus; partial strips are the `partial_strip` detector
  below — a `# repeat` emission would over-strip the untouched layers). Call sites inside
  `let` inits unwrap to their init call (T449 precedent — the emission
  stays a bare pattern and the engine rewrites the call sub-expression
  in place); `&`-led sides decline (general-expr territory);
  all-byte-identical classes stay for the tighter verbatim sweep (T422
  precedent). One `# allow=many` hunk (`- C(E0)` / `+ E0`, inferred
  hole, Section 15.8; uppercase last segments ride backticks per the
  call-sweep rule) with `# repeat` when some pair nested two or more
  deep — flat single-unwrap sweeps emit the plain hunk — so one hunk
  strips every depth bottom-up (Section 17.8). The plus side must hold
  no same-callee call (repeat would keep folding past the diff's
  post-state). Pre-image totality is the engine-count fold check: the
   real fold probe (`- C(E0)` / `+ E0` with `# sweep` + `# repeat`)
   must match exactly N outermost sites (nested matches are folds, not
   separate sites; macro/string twins never count — the engine never
   enters them). Detector name `repeat_fold`. At N = 1 a full-strip
   single fires as the allow-one fold template (tight original range,
   `# repeat` when nested two or more deep, plain hunk for a flat
   single-layer strip) under the precision guard: uniqueness is the
   same fold counter at n = 1 plus the T361 byte-compare with
   `# repeat`; an untouched outermost twin vetoes to the literal floor.
   N = 1 partial strips stay with `partial_strip`/ordinary.
- Partial-strip sweep (WI-1d): N ≥ 2 `-`/`+` pairs where every minus is
  the same 1-arg callee applied exactly one layer deeper than the plus
  (`C(C(x))` → `C(x)`; uniform depth both sides — mixed `2→1` + `3→2`
  classes stay ordinary, one template cannot spell both depths without
  overlap ambiguity), same callee discipline as the fold
  (leading `::`, qself, generics, `&`-led sides decline — narrower than
  the qualified sweep's engine spelling;
  all-byte-identical classes stay for verbatim, T422). `let`-init sites
  unwrap the same way. One plain `# allow=many` hunk with no `# repeat`
  (`- C(C(E0))` / `+ C(E0)` generalized to depth d, inferred hole;
  uppercase last segments backticked) — a `# repeat` emission would
  over-strip the deliberately kept layers (Section 17.8). Totality is
  the real-pair engine-count check (`template_pair_holds`): the emitted
  pair must match exactly N sites in the pre-image. Detector name
  `partial_strip`.
- Method-to-function sweep: N ≥ 2 `-`/`+` pairs, each a method call
  (`syn`-parsed, no turbofish, non-`&` receiver) → a bare
  single-segment call whose first arg token-equals the receiver and
  whose rest token-equals the method args, with one shared method
  ident, one shared function callee, and uniform minus-arity →
  one `# allow=many` hunk (`# expr Receiver` + `# expr E0…`).
  Uppercase idents ride backticks on the method
  (`` Receiver.`Old`(E0) `` — raw `Receiver.Old(E0)` binds `Old` and
  silently rewrites twin methods) and on an uppercase plus-side bare
  callee (`` `New`(Receiver, E0) ``).
- Method-to-method sweep: N ≥ 2 `-`/`+` pairs, each a method call
  (`syn`-parsed, no turbofish either side, non-`&` receivers) →
  a method call whose receiver and args token-equal per site, with
  one shared old-method ident, one shared new-method ident
  (`old != new`), and uniform arity → one `# allow=many` hunk
  (`# expr Receiver` + `# expr E0…`; uppercase idents backticked —
  `` Receiver.`Old`(E0) `` → `` Receiver.`New`(E0) ``, same hole
  class as the call sweep).
- Same-method literal sweep (T420): N ≥ 2 `-`/`+` pairs, each a method call
  (`syn`-parsed, no turbofish either side, non-`&` receivers, one trailing
  `;`/`,` stripped) → the same method ident with per-site token-equal
  receivers and cross-site token-equal old-arg and new-arg tuples with a
   real old/new delta → one `# allow=many` hunk (`# expr Receiver` only,
   concrete `ToTokens` spellings: backtick literals are ident-only per
   SPEC 15.4, and a plus-only `E` would be unbound; an uppercase method
   rides backticks — `` Receiver.`Old`(E0, lit) ``, same hole class).
   Pre-image totality
   keys on method + old-arg tuple (plain callee+arity would over-count
   same-method/different-arg sites that the pattern can never match).
   An arg hiding a macro invocation anywhere inside stays ordinary
   (WI-9b2-method-probe): TokenStream-normalized macro text is
   unmatchable by the engine, so the sweep declines and those pairs
   fall through to the nested macro sweep (or ordinary).
- `&`-receiver strip sweep: N ≥ 2 `-`/`+` pairs, each containing
   exactly one shared-reference-to-method call (`syn`-parsed, no
   turbofish/`&mut`/explicit lifetime) with the plus side equal to
   the minus with that span replaced by its receiver; one shared
   method ident and shared arg token-texts (typically literals, bound
   verbatim so `&d.join("repo")`-style sites can never over-match) →
   one `# allow=many` general-expr hunk (`# expr Receiver` only;
   an uppercase method rides backticks — `&Receiver.`Old`(argv)`).
   Trailing `;`/`,` tolerated.
- Qualified-call sweep: N ≥ 2 `-`/`+` pairs, each a multi-segment
  (`syn`-parsed, generic-free) `P::Old(args)` →
  `P::New(args)` call with a shared prefix (leading `::` shared
  verbatim — `::p::Old` renames compress; a bare pattern never
  matches a `::` site and vice versa, probed parity), an optional
  shared `qself` qualifier (`<T as Tr>` token-equal, never resolved —
  divergent qualifiers never fuse), one shared last-segment delta,
  uniform arity, and per-site token-equal args → one `# allow=many`
  hunk with the concrete path form (`- crate::old_api(E0…)` /
  `+ crate::new_api(E0…)`; SPEC 19.3 prefers concrete paths, no `# symbol`
  tax). An uppercase last segment rides backticks on that segment only
  (`` crate::`Old`(E0) `` — bare `crate::Old(E0)` leaves plus-side
  `New` unbound; only the last segment can infer, so prefixes stay
  verbatim). Bare single-segment `::foo` renames stay ordinary (no
  prefix to anchor the win). Call sites inside
  `let` inits unwrap to their init call (T449 precedent). Pre-image
  totality keys on the exact rendered path (qualifier + leading +
  segments) + arity (bare twins never match a qualified pattern and
  vice versa — probed parity).
- Turbofish sweep: N ≥ 2 `-`/`+` pairs, one uniform variant per hunk —
  bare calls (`foo::<u8>(x)` → `bar::<u8>(x)`), qualified calls
  (`crate::old::<u8>(x)` → `crate::new::<u8>(x)`, same prefix rule as
  the qualified sweep), or method calls (`recv.old::<u8>(x)` →
  `recv.new::<u8>(x)` with per-site token-equal receivers) — every side
  carrying `::<…>` (the `::` spelling; `let`-init calls unwrap, T449).
  Value args token-equal per site with uniform value arity; turbofish
  uniform turbo-arity with per-site minus token-equals plus; across all
  sites at most one turbo position varies — that position emits an
   inferred `T` on both sides (`- foo::<T>(E0)` /
   `+ bar::<T>(E0)`), the rest verbatim. An uppercase bare/qualified
   callee rides backticks (`` `Old`::<u8>(E0) ``,
   `` crate::`Old`::<T>(E0) `` — bare `Old::<u8>(E0)` leaves plus-side
   `New` unbound); turbofish-suffixed method idents stay plain (the
   `::<` suffix defeats hole inference — probed literal, no backticks).
   A bare-without-turbofish
   pattern never matches a turbofish site (R3 parity, probed), and
  turbo-arity is match-exact, so totality keys on callee + value arity
  + turbo arity + concrete non-hole turbo texts (`&`-receivers counted
  conservatively). Leading `::` compares with parity on the bare and
  qualified variants (shared verbatim). Two varying positions,
  per-site turbofish deltas, non-type holes, and qself-turbofish stay
  ordinary (a `qself` qualifier with `::<…>` has no template spelling).
- Macro-rename sweep: N ≥ 2 `-`/`+` pairs, each a bare
  (`old!(body)` → `new!(body)`) or qualified
  (`prefix::old!(body)` → `prefix::new!(body)`, one uniform class per
  hunk — shared prefix, one shared last-segment delta, generic-free
  segments, no leading `::`) macro invocation (whole-line statements
  and `let`-init invocations, which match the pattern in place —
  probed) with one shared old/new path and per-site token-equal
  bodies (bodies vary across sites; delimiters unrestricted — the
  engine `...` matches `!()`, `![]`, `!{}` alike, probed, including
  for qualified paths) → one `# allow=many` `# macro=token_tree`
   hunk (`- old!(...)` / `+ new!(...)`, or the concrete qualified
   form `- prefix::old!(...)` / `+ prefix::new!(...)`; SPEC 19.3
   prefers concrete paths, no `# symbol` tax). Uppercase macro names
   stay plain (macro names never infer — probed clean, no backticks).
   Token-tree matching is
  verbatim-only by design, so the body is never spelled. Leading-`::`
  (no engine spelling, probed), varying-across-sites hetero body rewrites
  and macro-body-descent interiors (WI-9b2), and divergent new names stay
  ordinary. Pre-image
   totality counts every invocation of the exact rendered path (any
  delimiter/body — the `...` pattern matches all; bare twins never
  match a qualified pattern and vice versa, probed).
  Rename + hole (WI-17a): when the class shares arity ≥ 2 under one
  uniform delimiter and exactly one top-level argument position varies
  across sites (rename-clean per site — the path carries the delta,
  never the args; stable positions token-equal everywhere), that
  position rides one diet `E{i}` hole beside the concrete stables
  (`- old_m!(E0, 1)` / `+ new_m!(E0, 1)`, no length gate — the hole
  names the varying position even for single-char values; strings never
  hole). Totality is the real-pair probe over the holed pair (tighter
  wins; any veto — multi-hole, strings, per-site deltas, mixed
  delimiters, unmatched twin — falls back to the verbatim `...` with
  its own probe).
- `macro_rules!` definition + invocation rename (WI-7b + WI-16): exactly one
  column-0 `macro_rules! Old { … }` → `macro_rules! New { … }` header
  pair plus N ≥ 1 bare `Old!(…)` → `New!(…)` invocation pairs sharing
  one ident delta → one def hunk with `# follow=uses`
  (`- macro_rules! Old` / `+ macro_rules! New`, WI-16 slice 1,
  byte-identical to the authored form). The engine's `macro_paths`
  follow rewrites every invocation by path identity; totality is the
  scratch line-equality probe over the exact site set (an untouched
  twin the follow would rewrite fails the unchanged leg, a missed
  site fails the plus leg). Bare invocations only (qualified
  `crate::Old!` stays ordinary); twin definitions, indented
  (block-level) definitions, and untouched invocation twins veto to
  ordinary; invocations nested inside another macro's body are opaque
  (`macro-hidden`: veto loud without fallback, skip with it — never
  rewritten, never silent). When the follow probe declines (no
  pre-image), the WI-7b two-hunk fallback emits instead sharing
  `rule=rename_Old_to_New`: the definition as a plain item hunk plus
  the invocations as one `# allow=many` `# macro=token_tree` verbatim
  sweep, under the same def-site + `macro_holds` proof plus a
  scratch-apply probe of the combined emission.
   Def-only singles (WI-19b, N = 1 definition pair, no invocations):
   the definition hunk alone in the concrete directive-free spelling,
   under the precision guard; same def-site proof + scratch probe.
   Detector name `macro_rules_rename`.
- Macro-interior sweep (WI-9a): N ≥ 2 `-`/`+` pairs rendering (whole-line
  statements and `let`-init unwraps — the emitted pattern is the bare
  `name!(args)`, never `let`) to one identical same-name pair —
  exact rendered path (bare or qualified, one uniform class per hunk),
  the same delimiter on both sides (paren, bracket, or brace —
  WI-9b2-delim; the engine splice emits with the plus pattern's
  delimiter), body delta, no holes — → one `# allow=many` hunk
  (`- name!(args)` / `+ name!(new_args)`, plain `!` routing, no
  `# macro` directive, first-site raw spacing). Cross-hunk driver with
  the verbatim stranded join (exactly-1-minus pure unbalanced hunks
  whose first plus matches shed minus + first plus and stay a
  pure-insertion leftover), so sections with a pure addition still
  compress their runs. Totality is the generic engine-count probe on
  the emitted minus (the engine's own match domain).    `![...]`/`!{}`
    Mixed-delimiter pairs stay ordinary (no cross-delimiter class);
   renames (name delta) and varying-across-sites macro-body-descent
   edits (changed lines nested inside macro bodies whose enclosing
   texts differ per site) stay ordinary; identical-enclosing descents
   group one level up (enclosing sweep below). Varying-across-sites hetero body rewrites
   group cross-site (WI-9b2 below), while pairs whose delta itself
   varies across sites stay ordinary. WI-9b1 hetero emission: when every
  stable (minus==plus) non-string arg longer than its hole name can hole
  to a shared diet `E{i}` hole (`E0` first, bumped past idents in the
  surviving literals/path so SPEC 15.7 equality never fuses hole to
  literal) and the shorter hetero pair passes its real-pair totality
  probe, the hunk emits hetero (`- assert_eq!(E0, TEXT_SECTION)` /
  `+ assert_eq!(E0, TEXT_STRIPPED)` — diet, no declarations: the
  both-sides inference rule gives `E0` → `expr` while single-side
  `TEXT_*` stay literal deltas); otherwise the concrete probe decides
  and concrete fires. String-literal args never hole (opaque positions);
  numeric/unit args no longer than the hole name stay literal (tighter +
  shorter); mixed delimiters stay ordinary as above. Totality for
  hetero pairs is the real-pair probe (`template_pair_holds`: the emitted
  minus/plus diet pair applied to a scratch pre-image must match exactly
  N — a self-rewrite probe would infer literal uppercase deltas as holes
  and count a more general domain than the hunk matches).
   WI-9b2a: the concrete fallback probes the real pair too (same
   soundness — the self probe holed shared uppercase deltas and vetoed on
   second-arg twins a sound concrete group must ignore).
   WI-9b2b nested extraction: pairs the whole-line parse declines whose
   lines parse as one `Stmt` (fallback `Expr`) holding the same count
   (≥1) of expression-position macros, all token-equal except exactly one
   delta with the same rendered path, the same delimiter, and different
   tokens, with the rendered delta texts each occurring byte-exact once
   per line and the plus line equal to the minus line under that one
   substring replacement — → the same concrete hunk shape spelling just
   the macro pair (`- env!("OLD")` / `+ env!("NEW")`, concrete-only, no
   hetero, no stranded claims). Macros inside other macro bodies stay
   ordinary (`syn::visit` never enters opaque token streams); nested
   renames, multi-delta lines, and any outside-macro difference stay
   ordinary.
   Nested multi-token bodies fire: the delta pair renders from raw
   source bytes via the nested nodes' `Span::byte_range` slices
   (WI-9b2 nested-raw) instead of the TokenStream grouping key
   (`1, 2` → `1 , 2`, which never occurs byte-exact in the line), so
   there is always a byte-exact substring to anchor on; slicing
   failures fall back to the normalized rendering     (single-token bodies
   render identically either way). WI-9b2 cross-site hetero grouping:
   leftover balanced pairs (singletons the identical classes veto)
   sharing path, delimiter, and arity group when every top-level arg
   position is stable in ALL pairs (token-equal everywhere → literal;
   varying → one shared diet `E{i}` hole, strings veto) or a delta in
   ALL pairs with the same trimmed `(minus, plus)` everywhere
   (→ literal delta); mixed stable/delta positions, varying deltas,
   and arity mismatch stay ordinary. Same engine token-wise matching
   (first-site raw separators), same real-pair totality
   (`template_pair_holds`, N = members), same index-based hunk
   atomicity, plus a per-group byte sanity so a losing candidate can
   never flip the section T375 into a whole-sweep veto.
   Enclosing-macro sweep (WI-7a): N ≥ 2 balanced pairs whose changed
   lines are not themselves macro invocations (whole-line and
   `let`-init shapes belong to the interior sweep above, which runs
   first) but sit each inside one same-spelling macro invocation in
   the pre-image — same path, same delimiter, byte-identical enclosing
   span texts on both sides after line-splicing each pair's changed
   line into its span (nested same-line macros never qualify: byte
   containment requires the span to cover the whole changed line).
   → one `# sweep` hunk spelling the raw enclosing texts
   (`-`/`+` bare prefixes, no added space — the span text carries its
   own indentation and the parser strips exactly one prefix char).
   Emission is always file scope: the candidate (enclosing span)
   exceeds any interior-line range, so a ranged hunk could never
   contain it. Totality is a multi-line engine-count probe
   (`template_enclosing_holds`: the emitted pair must match exactly N
   sites file-wide — an untouched twin vetoes). Same index-based hunk
   atomicity, same section-global T375 (no stranded joins in v1; tiny
   templates lose to ordinary hunks by design). `macro_rules!`
   definitions never enclose (WI-7b owns that kind).
- Rule-body interior sweep (WI-17b): N >= 2 arm-line pairs whose
  textual macro sets match in count (>= 1) with every position sharing
  path + delimiter and exactly one position differing in full text
  (each delta text occurring exactly once per line, plus line equals
  minus with that one substring replaced) group by identical rendered
  inner pairs (`- inner!(1)` / `+ inner!(2)`, concrete-only, no
  hetero, no stranded claims) with the same real-pair totality
  (`template_pair_holds`, now proving via the rule-body engine half),
  hunk atomicity, and section-global T375 as the interior sweep.
  Whole-line macro pairs never key here (the interior sweep owns them
  -- disjoint by construction); multi-delta lines (e.g.
  `outer!(inner!(1))` to `outer!(inner!(2))`, where outer and inner
  both differ) veto as depth-2 and stay ordinary. Detector name
  `macro_rule_body`.
- Use-path prefix sweep: N ≥ 2 `-`/`+` pairs, each a `use` statement
  (leading `pub…`/`::`, trailing `;` normalized away) sharing one
  two-or-more-segment prefix with a common parent and one renamed
  last segment, with per-site textually identical non-empty tails
  (plain/grouped/glob/`as` tails may mix) → one `# allow=many` hunk
  (`- use <parent>::Old` / `+ use <parent>::New`, no metavars).
- Use-regroup merge: N ≥ 2 `- use <prefix>::<Tail>;` lines plus one
  `+ use <prefix>::{<members>};` line whose tree is exactly their
  merge (same paths, grouped at the longest common prefix, members in
  minus order, uniform visibility, no attributes) → one
  directive-free hunk (no `- I` cut ever) under the precision guard
  with the D13 scratch-apply probe as totality (detector
  `use_regroup`; `as`/glob/nested members, vis-mixed,
  attributed, duplicate, non-adjacent, and single-minus shapes stay
  ordinary).
- Use-regroup split: one `- use <prefix>::{<members>};` line plus N ≥ 2
  `+ use <prefix>::<Tail>;` lines stating its expansion (same members in
  the same order, uniform visibility, no attributes) → one
  directive-free hunk under the same precision guard with the same D13
  probe as totality (detector `use_split`; merge↔split round-trips
  canonically; `as`/glob/nested members, vis-mixed, attributed,
  duplicate, and wrong-arity shapes stay ordinary).
- Whole-item move inference (T430/T432a/T432b/T432c/T432d): a byte-identical named item (fn,
  struct, enum, trait, type, mod, union, const, static) deleted in
  one file and added in another (or the same file, or a new file) compresses
  to `# item I` + `# bind` cut hunks spliced at the consumed pure-minus hunk
   plus `# use` paste hunks spliced after the rewritten pure-plus hunk (1b8993a:
   four fns move files, 873 ordinary lines -> 117). v1 gates: named items only
   (ident-less impl/use/macro stay ordinary; content-bearing inline `mod`
   (≥1 nested item) and method-bearing `trait` (any trait item) stay
   ordinary — the scalar `- I` cut would match the nested items too;
   empty `mod {}` / `mod;` / empty trait still move),
  global N-minus + M-plus pairing (one cut per surviving minus item sharing
  its bind across its pastes; duplicate plus-run claims dedup to the first
  pair in document order, the rest stay ordinary), cut-file pre-image flat-ws uniqueness (twin
  veto), minus paired runs anywhere with exactly one blank between runs (a cut
  absorbs exactly one following blank; the last run's absorbed blank must lie
  inside the minus hunk — a file-only following blank is a kept line the
  cut would silently eat, so the gate caps to the sound prefix instead (WI-6);
  without an absorbed blank a multi-pair last cut or a blank-ending prefix
  double-claims the preceding blank (f09edcd); uncovered
  prefix/trailing minus lines stay as rewritten ordinary hunks around the
  cuts, one post-run blank absorbed, never re-emitted), interior blanks ride
  with their item (the cut removes the whole span, so covered lines need no
  gap accounting), plus paired runs
  anywhere with exactly one
  blank between runs (the same-anchor paste join restores exactly one;
  uncovered prefix/trailing plus lines stay as rewritten ordinary hunks around
  the pastes), new-file pastes and deleted-file cuts allowed (T432d; the
  rebuilt prelude keeps the deletion, so the file still goes away), rename
  sections skipped, multi-cut hunks (distinct minus items; fan-out pastes of
  one item need no cap) whose last run has no following blank cap to N-1 cuts
  (the last run stays an ordinary trailing leftover; a
  single item with a blank-ending prefix and no following blank trims the
  absorbed blank from the prefix leftover hunk), and a global T375 byte-win
  guard (strict `emitted < replaced`, ties stay ordinary — e.g. a 3-line
  fn with short lines, 6 ordinary lines, costs more as cut+paste and
   stays ordinary by design; emitted cut+paste is ~constant (the item
   rides a metavar, never respelled) while replaced ordinary grows with
   content, so the same 3-line shape with a longer body fires — no fixed
   line threshold). A mixed `-U0` hunk (an insertion adjacent to an edit,
   with no unchanged line between them) splits into ordered pure runs
   for pairing (WI-6 slice 3a): each run cites exact walked anchors, so
   a trailing-edit plus run anchors past the cut and fires while a
   genuinely interior insertion still vetoes the section (the anchor
   inside the cut's claimed range is unorderable); two runs of one hunk
   never pair (same-spot edits stay with the intent detectors), and a
   group no pair touches re-emits verbatim;
  inter-run gap relaxation stays vetoed (the same-anchor paste join
   restores exactly one blank); WI-8a pairs `fn` moves whose header prefix
   (vis + qualifiers, verbatim) and/or name evolve while attrs, generics,
   params, return, `where`, and bodies stay token-equal — the paste carries
   the plus-side header template `+ <prefix>fn <New>(...) { ... }` and the
   engine splices the captured body under it (identical pairs keep the
   bare-`bind` paste); WI-8 slice 9 pairs `fn` moves whose signature
   additionally evolves (generics, params, return, and/or `where` differ
   while attrs and bodies stay token-equal) — the paste carries the
   concrete plus-side header `+ <prefix>fn <New>(args) -> ret { ... }`
   (single- or multi-line: a multi-line plus header rides verbatim
   across lines with only the `{ ... }` marker closing it)
   and the engine replaces the captured header span with it (both
   sides must open the block with a header line ending in `{` with
   body lines after it; single-line items stay
   ordinary); WI-8 slice 1 pairs `struct` moves whose visibility
  and/or name evolve while attrs, generics, and fields stay token-equal
   (paste `+ <vis>struct <New> { ... }`); WI-8 slice 2 pairs `enum`
   moves whose visibility and/or name evolve while attrs, generics, and
   variants stay token-equal (paste `+ <vis>enum <New> { ... }`); WI-8
   slice 3 pairs `union` moves whose visibility and/or name evolve while
   attrs, generics, and fields stay token-equal (paste `+ <vis>union
   <New> { ... }`); WI-8
   slice 4 pairs `type` moves whose visibility and/or name evolve while
   attrs, generics, and the aliased type stay token-equal (paste
   `+ <vis>type <New> = ...;`); WI-8
   slice 5 pairs `const` moves whose visibility and/or name evolve while
   attrs, the const type, and the initializer expr stay
   token-equal (paste `+ <vis>const <New> = ...;`); WI-8
   slice 6 pairs `static` moves whose visibility, name, and/or
   mutability evolve while attrs, the static type, and the initializer
   expr stay token-equal (paste `+ <vis>static [mut ]<New> = ...;`,
   with the plus-side `mut` riding verbatim like `fn` qualifiers); WI-8
    slice 7 pairs `mod` moves whose visibility, name, and/or `unsafe`
    evolve while attrs stay token-equal and content is absent (`;`) or
    empty (`{}` — any nested item vetoes, the scalar cut cannot isolate
    it) (paste `+ <vis>mod <New> { ... }`, with `unsafe`
    riding verbatim in the prefix like `fn` qualifiers; a `;`-vs-braced delta vetoes);
    WI-8     slice 8 pairs `trait` moves whose visibility, name, and/or
    `unsafe` evolve while attrs, generics, and supertraits stay
    token-equal and both sides declare empty traits (any trait item
    vetoes, the scalar cut cannot isolate it — same narrowing as
    `mod`) (paste `+ <vis>trait <New> { ... }`, with `unsafe`
      riding verbatim in the prefix like `fn` qualifiers; any `auto`
      token or `impl Trait for` restriction vetoes).
    WI-6 slice 3b pairs `fn` moves whose rename also renames the body's
    self-references (`fn fact` → `fn factorial` with `fact(n-1)` →
    `factorial(n-1)` in the moved body) while vis, qualifiers, attrs,
    generics, params, return, and `where` stay token-equal — the paste
    carries the body-evolving template
    `+ <prefix>fn <Old> as <New>(...) { ... }` (the `as` names the
    captured ident; the old spelling `+ fn <New>(...) { ... }` keeps its
    verbatim-body splice, so stale-self-ref and shadowed-local moves
    still fire unchanged) and the engine splices the header to the new
    name and renames word-bounded `<Old>` → `<New>` in the captured body
    through a comment/string/raw-string/lifetime-aware scanner, leaving
    string literals, char literals, lifetimes, and comments
    byte-verbatim. Occurrence law (WI-2): an occurrence rewrites iff
    `syn` shows it is the header ident itself or a self-recursive
    call/path; a shadowing local (or its uses), a field/method name, a
    macro name, or an attribute interior never rewrites — the authored
    paste vetoes with `RSMPL_PATTERN_CONFLICT` naming the kind and the
    captured line (rename by hand or split the move), and the raiser
    declines the same shapes to ordinary. The pairing proves the same
    scanner plus the same veto first (renamed minus item text == plus
    run text, raw string equality, no veto hit), so pairing and splice
    agree by construction; any non-rename delta vetoes to ordinary.
    WI-6 collection cells: one (minus hunk, plus hunk) cell holding ≥3
   distinct moved items that owns every pair of its minus hunk and every
   paired run of its plus hunk (no fan-out interleave, no split anchors)
   emits one ranged `# allow=many` + `# bind` cut plus one paste (bare
   `+ Mk` for byte-identical cells; WI-18 `N`-hole template
   `+ fn new_N(...) { ... }` for evolving cells where every member is
   abstract-evolved with the same kind/prefix and one uniform
   `pre_N_post` affix) when that spelling beats the scalar fan-in under
   T375, else the cell keeps scalar emission; the absorbed-blank gate
   above applies unchanged, except a source file emptied to zero bytes
   (minus hunk covering BOF..EOF) is sound for multi-item cuts (WI-18
   gate: no outside line to double-claim).
    WI-19 slice 3 pairs signature+body joint `fn` moves (WI-6 slice 3b
    rename plus WI-8 slice 9 signature evolution in exactly one slot —
    generics params, `where`, inputs+variadic, return; vis/name ride
    verbatim for free): the paste carries the joint template
    `+ <prefix>fn <Old> as <New>(concrete signature) { ... }`
    (single- or multi-line like the concrete form) and the engine
    replaces the captured header span with the new-signature header
    while renaming the captured body's self-references under the `as`
    occurrence law. The pairing proves the same veto plus raw body-tail
    equality after the rename, so pairing and splice agree by
    construction; zero-slot pairs belong to the `as` arm and two-slot
    pairs stay ordinary.
    WI-19 slice 4 pairs container body-evolving `struct`/`enum`/`union`
    moves (a rename whose field types / variant payloads name the
    container): the paste carries the per-kind `as` template
    (`+ <vis>struct <Old> as <New> { ... }`, likewise `enum`/`union`)
    and the engine splices the header while renaming the captured
    suffix/body's type-path self-references under the `as` occurrence
    law generalised per kind (field/variant ident, macro name, and
    attribute-interior vetoes). The pairing proves the same veto plus
    the renamed-minus == plus-run agreement, so pairing and splice
    agree by construction; vis/generics/attr deltas belong to the
    abstract path or stay ordinary, never here. WI-19 residue pairs
    inherent-`impl`-block body-evolving moves (a self-ty rename whose
    body self-paths follow): the paste carries
    `+ impl <Old> as <New> { ... }` and the engine splices the header
    while renaming the captured suffix/body under the same law with
    assoc-item-name vetoes (a method/const/type named `Old` lives in
    another namespace); trait-impls (`impl Tr for Ty`) stay ordinary
    (WI-3 owns that header).
    WI-14 slices 1-3 + slice 5 pair move-eligible item moves gaining exactly one
    single-line `///` outer-doc line or one balanced single `#[...]` attr
    (single-line, or multi-line up to the raiser cap) while every other
    slot stays token-equal: the cut + bare paste carry the move while
    the attr/doc lines ride an ordinary plus-prefix leftover hunk above
    the paste (the T432b prefix shape — no new spelling, no engine
    change).     WI-14 slice 7 extends the same prefix shape to `fn`
    move+rename with exactly one added leading block (single-line
    `#[...]` / `///`, one balanced multi-line `#[...]` up to the cap,
    or one added block atop an existing attr): the minus cut is bare
    (span law satisfied), the inner rename pairs through the abstract
    evolved matcher, and the paste carries the evolution template
    (`+ fn <new>(...) { ... }`) with the added block as the ordinary
    prefix. WI-14 slice 8 extends the same composition to every other
    move-eligible kind (`struct` / `enum` / `union` / `type` / `const` /
    `static` / `mod`, each through its own abstract evolved matcher and
    header-prefix helper; the trait finder is untouched): the paste
    carries the kind's evolution template with the added block as the
    ordinary prefix. WI-14 slice 9 extends the same composition to
    empty `trait` rename+attr (through `trait_evolved_match`, so pure
    attr-added without rename never double-claims; E2 slice 1 admits
    method-bearing traits with token-equal methods): the paste
    carries the trait evolution template with the added block as the
    ordinary prefix. WI-14 slice 10 closes the axis: the struct
    composition arm admits a vis delta riding with rename+attr
    (`struct Old` → `#[derive(Clone)] pub struct New` fires as cut +
    evolved paste + ordinary prefix — the stripped-inner evolved
    match needs only a vis-or-name delta, so the "container-delta
    beyond vis" residue for the added axis is landed, not deferred);
    E2 slice 1 fires the method-bearing trait bare twin
    (byte-identical move) and the header-only rename twins
    (`trait Old { fn foo }` → `trait New { fn foo }`,
    plus the rename+attr twin) via the token-equal-methods gate —
    the engine scalar cut isolates the whole trait
    (`outermost_spans`) and the splice keeps the body verbatim.
    E2 slice 2a fires the self-ref twin
    (`trait Old { fn make() -> Old }` →
    `trait New { fn make() -> New }`) as one intent with the
    `+ trait Old as New { ... }` paste (name-only scope, full-text
    agreement through the shared rename scanner, method/macro/attr
    E2 slice 2b fires the one-method-rename joint
    (`trait Old { fn m_old }` → `trait New { fn m_new }`) as one intent
    with the `+ trait Old as New { fn m_old as m_new; ... }` paste
    (name-only scope, full-text double-scan agreement, Old-side
    method/macro/attr vetoes plus the `m_old` macro/attr joint veto;
    all four idents pairwise distinct; exactly one method renamed —
    two-method renames stay ordinary). Attr+joint composition stays
    ordinary (the attr-added arm strips then calls the abstract
    matcher, which vetoes item deltas). Tiny `mod` and tiny-trait rename+attr stay ordinary under the
    T375 byte guard (a 1-line empty mod or empty trait plus one attr line can never
    price the cut+paste below the ordinary hunks), and content-bearing
    mods still decline via the shared cut gate. Two added attrs, `//!` /
    `/* */` docs, two-added+rename, and item-interior
    (`#[keep]+#[old]+fn` → `#[keep]+fn`, needs discontiguous cuts) attr
    deltas stay ordinary. Hunk-interior removed (one removed attr between
    companion cuts, bare or +rename) fires via the same E1 sub-span cut
    below with the removed block as an interior ordinary minus leftover
    between the cuts; hunk-interior replaced (one removed attr minus-side
    + one added attr plus-side between companions, bare or +rename) fires
    the same way with interior ordinary leftovers on both sides (the plus
    gap admits blank+single-attr, the rebuild interleaves it between the
    pastes). Removed / replaced
    bare at prefix position fires via the E1 sub-span cut: `- I` cites
    the stripped span (`find_stripped_item_target`, move-cut-only) with
    the old block as an ordinary minus prefix (removed) or ordinary
    prefixes on both sides (replaced) around a bare `+ M1` paste; the
    removed/replaced + rename twins fire the same way with an evolved
    paste (stripped remainder through the shared evolved cascade —
    abstract arm for removed+rename, plus-added arm for replaced+rename;
    first-of-N strip serves kept docs).
       Everything else (other item kinds, other attr deltas, vis+body combos
       without a signature delta, two-slot signature+body combos,
       trait-`impl` moves, shadowed-body
       renames the pairing scanner disagrees with, single-line items)
       stays ordinary: abstract, body-evolving, and concrete-signature
       multi-line headers fire, as do single- and multi-line
       signature+body joints (diffrs fixtures
       `137-multiline-abstract-move-fires`,
       `138-multiline-sig-evolve-fires`, and
       `139-sig-body-joint-fires`) and single-line container
       body-evolving `as` pastes (diffrs fixture
       `140-container-body-evolve-fires`) and inherent-`impl`
       body-evolving `as` pastes (diffrs fixture
       `141-impl-body-evolve-fires`).
- Whole-file item-deletion sweep (T464): a deleted `.rs` file whose
  minus lines tile the whole file and parse as Rust compresses to one
  `@@ -1,1 @@ rule=delete_items` hunk (`# scope=file`, `# allow=many`,
  `# item I`, `- I` — SPEC 18.2 bare deletion; the engine deletes every
  contained item) plus ordinary `@@ -s,l +0,0 @@` leftovers for non-item
  lines (module docs, comments, stray blanks; `\ No newline` markers ride
  their minus line). Overlapping pure deletions union-merge at plan time
  (each item delete absorbs one adjacent blank, so neighbours double-claim
  the shared separator — the union is still just a deletion), every other
  overlap pair still fails. Byte-win guarded (T375).
- Partial-file item-deletion sweep (T465): a minus-heavy unbalanced
  native hunk whose minus lines are whole items — top-level or nested
  (mod-nested fns, `impl`/`trait` methods, T466; the engine `- I` delete
  removes the same nested set, so every fire applies) — in the pre-image
  compresses to one real-range `@@ -O,L +N,0 @@ rule=delete_items_K`
  hunk (`# allow=many`, `# item I`, `- I` — default Contained scope
  deletes exactly the items in the range) plus one ordinary insertion
  leftover for the plus lines (with a compensating trailing `+` blank when
  the pre-image line after the range is blank — the last delete absorbs
  it). Edge-crossing items (an item straddling a range edge is edited,
  not deleted) veto; strict containers (`mod tests`, `impl Foo` around a
  nested delete) are fine. Leading in-range blanks ride an ordinary
  deletion hunk ahead of the shrunk semantic range (the engine absorbs a
  preceding blank only at EOF); two or more trailing in-range blanks veto
  (conservative). Balanced hunks keep their rename/call detectors
  (strictly minus-heavy only); mid-item cuts, non-item code lines,
  EOF-touching regions whose absorbed preceding blank is kept (T472: EOF
  items preceded by BOF, code, or an in-range minus blank fire), and T375
  losers stay ordinary.
- Verbatim identical-line sweep (T403): N ≥ 2 identical balanced
  single-line (`-`/`+`) pairs shared across a section's hunks (pure
  additions elsewhere veto whole-section balance but not this sweep)
  compress to one `# allow=many` hunk per group, with literal pattern
  lines and no metavars. Both sides strip one terminator and parse as
  `syn::Expr`; complete single-line `let` statements (T440) parse as
  `syn::Stmt::Local` and sweep on the full `;`-terminated lines (engine
  `Stmt` shape; multi-line fragments never parse and stay ordinary).
  Macro invocations on either side (SPEC 21), bare-ident/literal minus
  sides (expressions only — a whole-statement `let` pattern is always
  precise; hetero-frame lone-literal deltas are the `literal_frame`
  sweep's, above), and `?`-applied calls
  wrapping a closure (minus side), and top-level Unary on either side
  are rejected — each failed loud in-corpus (top-level Unary matches since
  T415: leading `!` skips the macro pre-route and leading `!`/`-` bypasses
  the call router to a general-expr hunk — b208847 compresses);
  inner-`!` routes call/general since T428 (only `name!` + body opener
  `(`, `[`, `{` with strings skipped is macro; placeholder-carrying
  concrete args bypass to the general matcher; arg splitters are
  string-aware);
  non-Call-top-level pairs
  with nodes outside the engine's bounded general-expr set
  (Block/If/... — Return supported since T418, b9c00d5) are rejected.
  Struct-literal fields (`label: expr`, T416) strip the label and sweep
  on the inner expression when both sides parse and labels match (c48a6e6).
  Soundness is subterm totality on the pre-image file
  (index blob for pending diffs, `rev` blob for staged/rev diffs):
  the stripped minus must occur as a sub-expression exactly
  group-size times, with zero occurrences inside opaque text (macro
  bodies, string interiors — the engine never matches there; line-aware
  (T411): a hit vetoes only when its span covers a changed old-side line).
  `let` groups count whole statements instead (T440): the full line must
  occur as a `Stmt::Local` exactly group-size times — parsed statements
  cannot hide inside macro bodies or string literals, so no line-aware
  veto applies. A stranded identical pair inside an unbalanced hunk
  joins its group instead of vetoing it (T441 slice): an exactly-1-minus
  pure `-`/`+` hunk whose first plus matches the group sheds minus +
  first plus and stays as a pure-insertion leftover (`@@ -O,0 +N+1,K-1 @@`);
  multi-minus, context-bearing, or first-plus-mismatch hunks stay
  ordinary (the leftover would split in two).
   Syntactic, not textual: `&s.p` inside `&s.p.items` is a different
   subterm and does not veto (probe-verified). Uppercase call heads
   ride backticks (`` `Old`(alpha) ``, `` a.`Old`(alpha) `` — a raw
   uppercase callee/method infers as a hole and rewrites twin sites);
   any other surviving uppercase ident (let-pat, general-expr
   positions) is judged by an engine self-rewrite count
   (`template_expr_holds`): the minus template must match exactly
   group-size times or the group stays ordinary (conservative:
   whole-file count, so an out-of-range twin still vetoes). Leftover balanced hunks
   retry the standard chain in place; the T375 guard applies to the
   retry the standard chain in place; the T375 guard applies to the
   verbatim-covered set as a whole. Hunk atomicity (WI-9e, same
  index-based fixed point as the macro-interior sweep): a group fires
  only if every balanced hunk holding a member pair is fully covered
  by passing groups — a group whose member hunk also holds an
  uncovered pair drops (releasing its stranded-join claims) to a fixed
  point, and uncovered leftovers stay ordinary; rules reserve only for
  survivors.
  N=1 literal single (WI-6 slice 1): a whole-line single no holed
  single owns raises as one directive-free allow-one hunk with literal
  `-`/`+` lines (the fragment is the whole matched unit), under the
  precision guard (`body <= hunk + 16`). Same gates as the sweep
  (shape, novelty, backtick emission), uniqueness proved at n=1 over
  the pre-image file, plus the T361 real-apply byte-compare
  pre-reservation; the rule is reserved only for emissions that
  reproduce. The single runs last in the per-hunk chain (after every
  holed single and the single-pair definition/perm shapes), so tighter
  spellings always win; `guard_for("verbatim")` stays Strict for
  multis. In subhunk runs a declined whole still tries the bounded
  split-search before per-pair singles, so literal keys never shadow
  the hetero multis their pairs belong to (bench 48). WI-6 slice 2:
  array-repeat (`[E; N]`, elem plus length structural) and `&raw`
  addr (mutability token-equal, pointee structural) join the shared
  shape gate — both match structurally in the engine since WI-2a, so
  the raiser admits them alongside every other structural shape;
  macro invocations stay vetoed (token-equal engine arm plus
  macro-body opacity, Section 21).
  All seven metavar sweeps carry the same pre-image subterm totality
  (T409): matchable-unit count (calls/methods by callee+arity, same-method
  literals by method+old-args, strips by method+args, slots by fn ident
  free vs impl, uses by prefix) must equal
  the pair count, with zero stripped-minus hits in opaque text — precise
  line-aware needles (T411: container spans must cover a changed old-side
  line; snippet callee checks over-vetoed 57140fe via assert!/patch-string
  literals).
- Everything else (multi-line edits, pure deletions/additions,
  non-`.rs` files, unbalanced hunks (whole-item moves pair pure-minus/pure-plus
  twins instead — see above; verbatim stranded-join pairs excepted),
  bare single-segment `::foo` renames and `qself`-turbofish call sites,
  non-strip
   `&`-receiver sites, single-segment use prefixes, macro
  interiors (same-name body rewrites), headers-only rename sections
   and renames with no compressible content (WI-7a: rename sections
   whose content hunks compress under any existing detector emit the
   semantic hunks against the new path inside the preserved rename
   section — the engine seeds rename content from the old side, SPEC
   6/T377 — with pre-image totality read from the old path (C-decoded via the shared quotepath rule, same as the parser); a rename
   with no compressible content stays passthrough byte-for-byte, T377) stays
   ordinary-hunk passthrough.
  Pure deletions/additions stay ordinary by design (T424 verdict — no
  separate negative/positive sweep vocabulary): positive fan-out is
  engine-impossible (a pure insertion anchors at one point, with no
  minus pattern for `# allow=many` to multiply over — probe-verified),
  and minus-only call sweeps lower to `;` litter (Section 18.2) while
  the whole-line negative shapes (let/item/use) have zero all-identical
  pure-hunk exhibits in history. Ordinary 0-context hunks already show
  each removed/added line at one line per site — the minimal view.

Fallback rule: the candidate patch must parse via `parse_patch`;
any validation failure falls back to the ordinary passthrough —
`diffrs` never emits an unparseable patch. One allowance law prices
competing risks (WI-19/WI-7, one decision point:
`rsmpl_diff_infer::semantic::guard_for` for the posture plus
`guard::precision_allowance` for the number, decided by the single
helpers `within_precision` / `within_strict` — whole-section
pair-chain emissions consult `guard::pair_section_gate`, which prices
Precision detectors under the allowance and keeps T375 for Strict).
Sweep shapes (calls, method sweeps, `general_expr`,
`control_template`, verbatim, macro interiors — any minus pattern
that could match wider than the diff)
keep strict T375 (`within_strict`: a semantic hunk is kept only when
byte-smaller — bytes proxy tokens, and bytes proxy over-match risk —
than the ordinary hunk(s) it replaces; whole-section candidates
against the whole section, per-hunk candidates against their hunk,
prelude shared either way; ties stay ordinary). Definition shapes
(`item_rename`, `member_rename`, `assoc_rename`,
`macro_rules_rename`, slot/sig/header evolution singles, definition
evolution/addition, `delete`, `move`, `mod_rename`, `item_intent`,
the two method follow intents (`trait_method_rename`,
`inherent_method_rename`) —
every detector whose plus the WI-1 postcondition re-verifies at its
spans) fire under the precision allowance (`within_precision`:
semantic ≤ ordinary + `precision_allowance(n_claimed, filter_extra)`
with `precision_allowance(n, extra) = 16 × max(1, n) + extra`; the
partial partition scales the allowance by claimed pairs and filtered
singles add their mandatory anchor-block bytes as `filter_extra`).
The ordinary side on both helpers is the rendering-normalized span:
git's elided `,1` prices like the explicit `,1` form at hunk and
section level, so the verdict never depends on header rendering.
The 16-byte constant is the price of the `diff --git` + `@@ … rule=`
+ pattern lines the ordinary form does not carry — a pinned SPEC
constant, never a tunable discovered in code. A losing shape falls
back to ordinary (whole-section losers still retry per hunk), so
sweep singles and tiny sweeps never cost more than the passthrough
(no fixed line threshold — the comparison is bytes, so a vetoed move
minus side may still ride a `delete_items` sweep firing on the same
minus run). Every output therefore
parses by construction; the pinned shapes additionally round-trip
through `applyrs --check` against the pre-change tree. Semantic
validity beyond the pinned shapes is best-effort: verify with the
D.3 round-trip recipe before trusting a compression.

History-per-commit regression recipe (T346): `checkout c` +
`diffrs c~1` converts commit `c` (worktree `== c` makes the rev
diff exactly that commit); the root commit has no parent, so it
converts against the empty tree (`git hash-object -t tree
/dev/null`). Round-trip each capture by materializing the parent
with `git archive c~1 | tar -x -C <tmp>` (an empty dir for root)
+ `applyrs --file change.rsmpl --check` (T356 isolation — no
second checkout, so the `applyrs` `.history/` audit write lands in
the throwaway pre-image instead of polluting the diffrs worktree).
The curated selection in `bin/diffrs/tests/history_e2e.rs` is the
pinned generator: every entry must convert and check clean with
its pinned semantic-hunk count, then real-apply byte-identical to
the child tree (T361: `--check` is blind to sweep over-match) —
failing conversions become pins there.
The repo-owned runner `tools/history_sweep.py` (T421) automates this
recipe over every commit and reports the compressing set.

In a full checkout a missing curated pin fails the harness (stale
recipe, not a skip); the root conversion ensures the empty-tree
object via `git mktree` first (T362).

Empty changes print empty stdout + `(no changes)` on stderr (never
pipe empties into `applyrs` — empty input is the T59 authoring
error). Failures exit 2 with the stdout-only `RSMPL_IO_CONFLICT`
envelope family (same as `applyrs`, Section D.2). JSON shape:
`{version, files: [{path}], patch}`.

### D.5 Canonical form and lossless cycles

Canonical RSmPL is what `diffrs` emits and what cycle comparisons
compare: diet spelling (tight ranges or `# sweep`, inferred
metavariables, terse renames), directives in fixed order, files in
canonical order with hunks in document order, one hunk per detector
class per file — except per-block impl/trait-context hunks (slot /
sig-evolution over K blocks), which share one rule name across K
hunks in document order. Two comparators share this form: pure
`rsmpl_patch::canonicalize` (parse → re-emit) preserves the authored
 locality spelling, while `rsmpl_engine::canonicalize_with_tree`
 matches every contained/file-scope hunk with engine-proven spans
 against the pre-image tree and re-emits it under the range rule below
 (tight when its sites span at most 64 lines, else the `# sweep`
 dummy) — so an authored sweep, an inferred ranged hunk, an inferred
 precision single, and a stale-cited hunk over the same sites share one
 form. Single-site hunks of either cardinality collapse to the diet
 directive-free tight actual range; multi-site `allow == many` hunks
 take the tight `@@ -min,len @@` with `# allow=many`. Two hunks compare
 equal iff their site sets, templates (post-diet), and file sets are
 equal: locality never distinguishes, templates always do. The
 tree-aware form also unifies symbol spelling: an authored
 `# symbol Old=crate::old_api` call hunk (`- Old(E)` / `+ New(E)`
 with `# fallback=syntax`) is rewritten to the concrete-path diet
 form the inferrer emits (`- crate::old_api(E0)` /
 `+ crate::new_api(E0)`), dropping the `# symbol` lines, the
 `# fallback=syntax` line (kept only beside `# follow=uses`, where it
is the WI-6a skip switch), and redundant `# expr` / `# type`
declarations (declared expr metavars rename positionally in
first-appearance order, `E` → `E0`, `E1`, …; declared type metavars
rename the first hole to bare `T` — the turbofish emitter's
single-hole spelling, `- foo::<T>(E0)` — and extras to `T1`, `T2`,
…, so `Old::<U>(E)` + `# type U` shares the inferred `::<T>(E0)`
form). Only single-pair bare-symbol call hunks normalize this way;
 method shapes, `let` wrappers, multi-pair bodies, and hunks without
 `# fallback=syntax` keep their authored spelling (their error shapes
 must survive comparison, never silently heal). The `let` diet
 likewise unifies single pairs and WI-5c sequences: an authored long
 spelling with declared `# pat`/`# type`/`# expr` metavars over one
 `let` pair — or K = 2–4 `let` lines per side — renames positionally
 across the whole hunk (`P` → `P0`, `T` → `T0`, `E` → `E0`,
 second-position names onto `P1`/`T1`/`E1`, …) and drops those
 declarations, matching inferrer emission; count-mismatched sides,
 non-`let` lines, and any token collision keep the authored spelling.
 Undeclared diet-short holes normalize the same way: a `let` pair or
 K = 2–4 `let` sequence carrying no declarations at all but spelling
 bare single-letter holes (`Some(P)`, `E`) renames them positionally
 by conventional kind (`P`/`Q` → `P0`/`P1`…, `T` → `T0`, `E` → `E0`)
 — the pattern compiler infers them as holes (Section 15.8) and the
 engine applies them as such, so the undeclared spelling (fixture 84)
 shares the canonical form of the declared (fixture 73) and inferred
 diets. Multi-letter undeclared names never synthesize (kind
 inference the comparison must not guess); the inferrer leg, already
 positional, is a no-op under the ident-boundary matcher.
 The template diet unifies the same-site spellings the raiser never
 emits: an explicit default-valued `# lang=rust` line drops (the
 inferrer never writes it; kept beside an explicit `# mode`
 directive, whose interaction with `lang` stays pinned); a declared
 `# expr` hole in a single-pair call/method/general-expression hunk
 renames positionally to the inferrer diet (`E`/`Arg` → `E0`, … by
 first appearance) while inferrer diet vocabulary (`Receiver`,
 `E0…`/`P0…`/`T0…`, bare `T`, `V Q G W N I`) stays verbatim, and all
 `# expr` declarations drop on success — so an authored
 `Receiver.old_method(Arg)` and an inferred
 `Receiver.old_method(E0)`, or an authored `E + 1` and an inferred
 `E0 + 1`, share one form (symbol hunks, turbofish pairs, `let`-led
 or `fn`-header lines, multi-pair bodies, context lines, unmentioned
 declared names, and token collisions keep the authored spelling);
 and a framed fn pure-rename pair (one minus/plus pair of `fn`
 headers, optional equal `V Q ` prefixes, token-identical outside
 differing names, only `...`/`{`/`}` context) contracts to the bare
 `- fn Old` / `+ fn New` diet — so terse-framed, explicit-framed, and
 bare legs over the same sites compare equal (evolutions, concrete
 vis, multi-pair bodies, the `N` name hole, and stray context keep
  the authored spelling). A multi-site `# follow=uses` hunk under the
  default `allow == one` normalizes like `allow == many` (tight actual
  range + `# allow=many`, comparison-only): follow spans are def +
  follow sites by construction, so an authored ranged follow and an
  inferred sweep follow over the same sites share one form. Anchors,
  non-follow `allow == one` hunks with ≠1 proven spans, ordinary hunks, non-contained/file scopes, and
  hunks with no matched spans keep their authored ranges. Hunks keep
document order rather than range-sorting: an insertion anchor at a
range edge applies in document order (Section 20.2 — a paste at
`(1,0)` plus an edit of `[1,1]` yields different trees when swapped,
probed via the 03-move-fn shape), so range-sorting would change
application results. Emission
range rule: if all sites of a class lie within one contiguous
original range of at most 64 lines, emit that real range with
`# allow=many` (no `# scope=file`); otherwise `# sweep` (dummy
`@@ -1,1 @@`). Moves and deletes keep today's real ranges/anchors.
Tree-aware comparison derives the same rule from matched spans: the
union of a hunk's engine-reported site spans (`min` to `max`) decides
— at most 64 lines cites that tight range (directive-free when the
union is one site, `# allow=many` otherwise), wider spreads become the
sweep dummy; when the patch fails to apply (zero-match, overlap,
unparseable target) comparison falls back to the pure form over the
spell-normalized patch (ranges stay authored, symbol spelling still
unifies), and the
tree legs of the harness still pin real breakage. Rule ids are `r1…`
in
canonical hunk order, except cross-file shared intents
`rename_Old_to_New` (WI-7b), which keep their descriptive names —
making the form deterministic: the same
pre/post trees yield a byte-identical canonical patch.

Concretely (`rsmpl_patch::canonicalize`, parse → re-emit):

- Files in canonical order: path-sorted, then per shared `rename_*`
  group (a rule name occurring in ≥2 hunks) the containing file
  subsequence is stable-sorted definitions-first — a file whose grouped
  hunk minus line is an item header (`fn`/`struct`/`enum`/`trait`/
  `type`/`mod`/`union`/`const`/`static`/`impl`/`pub`/`V`/`Q`) before
  call/use/macro-site files, ties by path; ungrouped-only patches
  reduce to path-sort. Hunks within a file keep document
  order (insertion anchors at a range edge are order-sensitive,
  Section 20.2 — range-sorting is not application-safe). Ranges cite
  original pre-patch coordinates (Section 10).
- Semantic rules renumbered `r1…` in canonical order (one global
  counter, matching the `diffrs` emitter convention), except shared
  `rename_*` names, which are preserved verbatim and consume no counter.
- Directives in fixed order: `# sweep` (fused exactly when effective
  `allow == many` and `scope == file`, else the explicit non-default
  `allow`/`scope` lines); `# optional`; `# repeat`; `# lang=rust`;
  `# root=`; `# bind=` then `# use=`; `# symbol N=P`;
  `# <kind> N1, N2` merged per kind, kinds sorted, names deduped;
  `# attrs=/# mode=/# macro=/# format=` only when non-default;
  `# returns=none`; `# fallback=syntax`; `# metavars=sigiled`.
  Effective values are emitted, so positional `# sweep` expansion
  re-emits identically.
- RSmPL headers as `@@ -old_start,old_len @@` (new side elided,
  Section 10.4); with `# scope=file` the range is the canonical
  `@@ -1,1 @@` dummy (Section 17.3) except zero-length insertion
  anchors, which ignore scope (Section 10.5). Ordinary hunks keep
  both ranges; `@@ … @@` titles are dropped.
- File headers collapsed to single-path `diff --git <target>` when
  Section 6 allows it (rename/create/delete halves keep two paths);
  `index …` lines dropped (the lowering never reproduces them);
  all other metadata and all body lines kept verbatim.

Losslessness is judged on file trees plus canonical patches, never
raw native bytes: the 0-context lowering cannot reproduce an
arbitrary hand-written native patch byte-for-byte, and `index`/title
stripping is deliberate. Equalities per origin close the contract:

- Native → semantic → native (state-lossless): applying the lowered
  native patch to the pre-tree yields a post-tree byte-identical to
  the true post-tree.
- Semantic → native → semantic, multi-site (canonical-lossless):
  re-diffing the post-tree and re-inferring yields `S′` with
  `canonicalize_with_tree(S′) == canonicalize_with_tree(S)` under the
  pre-image, and one further iteration is a fixed point
  (`S″ == S′` byte-identical). Anchored fixed point (WI-5/D10):
  where an anchored general template and a bare literal prove the
  same site set, `S′` is the anchored spelling and an authored bare
  form re-diffs to it (like terse→sweep normalisation).
- Semantic → native → semantic, single-site (literal-instantiation
  equality, D17): at exactly one distinct line site, an authored
   templated, holed, or symbol spelling and the inferred literal or
   holed diet single compare equal iff both instantiate to the same
  literal pair — the trimmed pre/post lines at that site
  (expression shapes terminator-stripped per the raiser precedent,
  statement shapes whole; bare-type hunks instantiate to the concrete
  type delta between the site lines, since a lone header line is not
  an applicable pattern) — with locality and hole declarations
  dropped, and a unified-patch re-apply rebuilds the identical
  post-tree (multi-subsite lines unify at line granularity: one
  distinct line span is one site). Over-wide templates (two line
  sites vs a one-line literal), hetero multi-site shapes (e.g. a
  two-site let-verbatim macro sweep vs a sub-line call sweep), and
  uppercase-head spellings whose plain-literal representative does
  not apply (bare uppercase heads infer as holes) stay canonically
  divergent with their reasons named; the tree legs still hold and
  one further iteration is a fixed point (`S″ == S′`
  byte-identical).
  Filtered fn-rename hunks (`impl`/`trait` filter plus `(...)`-family
  pair) instantiate instead to the shared concrete inline form
  `fn {name}(...) { ... }` for definitions and `fn {name}(...) ;`
  for trait declarations (`{` vs `;` never unify), frame dropped.
  Macro-rename hunks (`# macro=token_tree` plus one minus/plus pair)
  instantiate through the verbatim arm, never the type-delta arm (a
  macro invocation body parses as a type, so the type arm would carve
  a callee fragment that never re-proves): both legs rewrite to the
  terminator-stripped expression pair (`old_m!(a, 1)` — a
  `;`-terminated concrete macro pattern is rejected outright,
  `NAME!(...)` shape only — so an authored verbatim `...` leg, an
  authored holed leg (`E` undeclared), and the inferred allow-one
  verbatim single share one literal form, `# macro=token_tree` kept
  on every leg; a macro leg and a non-macro leg over the same site
  keep different directives and stay divergent.

Corpus in scope: all applyrs fixtures (121), all diffrs fixtures
(144), all example benches (32 applyrs, 48 diffrs), plus the history_e2e pins. Out of scope
(declared lossy): binary/gitlink sections (stripped with a warning),
non-UTF-8 sections, and CRLF semantic compression (whose passthrough
cycle must still be tree-lossless).

Property harness (WI-19 slice 1, `crates/rsmpl_diff_infer/tests/property_cycle.rs`):
seeded native-side mutations (fn rename, call swap, control-header decoy, N=1 single)
raised via `infer_rsmpl_with_pre_images` must parse, apply to the native post tree
byte-identical, and re-infer byte-identical; hetero `&`/field-arg sweeps are excluded
until emission spacing fidelity lands (DISCOVERED TASKS.md item).
Property harness (WI-19 slice 2, `crates/rsmpl_diff_infer/tests/property_cycle_authored.rs`):
seeded authored-side mutations (fn rename, call sweep, `# optional` zero-match, `# repeat` fold, stale-range, overlap veto) applied via `apply_patch_to_tree` must never panic, Ok must deterministically reach the independently recomputed post, Err must carry a known SPEC 25 code.

Exclusions (canonical divergence the `cycle_e2e` harness pins, not
bugs in the cycle itself): single-site shapes the raiser
deliberately leaves ordinary (`13`,
`22` (rename + full attr/doc strip is a two-slot joint — WI-8
composed-probe territory; no D17 arm admits `Attrs`-carrying hunks
by design) — other definition-shape singles among them fire under
the WI-19 precision guard wherever the single-pair gates admit them;
macro-rename singles fire as allow-one verbatim `old!(...)` /
`new!(...)` hunks under `# macro=token_tree` (one invocation,
token-equal bodies, `macro_holds` at n=1 plus the scratch
byte-compare; holed authored legs unify via the verbatim literal
arm); same-name macro-interior edits stay with the interior family
(`75`: the whole-line let-with-macro literal matches nothing in the
engine — T440 wall — so D17 re-prove cannot keep; needs engine
literal-statement matching with macro inits);
return-carrying/vis-carrying fn singles still need their slice-2
detectors); `04` divergent mixed sections (one hunk re-infers
semantic while the ordinary remainder never normalizes): hunk-2
raises as a `call_sweep` single while hunk-1's one-line col0 fn
rename stays ordinary (slot frame ~2x native, precision-guard veto;
terse admission reverted — greedy partial would park hunk-2's win;
opener is WI-6 free-fn col0 emission with leftover retry);
heterogeneous sites no template fires on stay
  ordinary (interleaved same-class groups — A B A line order stays
  ordinary, order is never manufactured; container-joint residue without
  uses, e.g. the first hunk of `99-partition-partial-mixed` — and no
  header+body joint shape in a single pair. Mixed hunks whose runs all
  prove now split per WI-4 (diffrs fixture `129-subhunk-partition-fires`),
  and fused `fn`-header-plus-statement def hunks split per WI-15 slice 1
  (Joint-A/rename-only header follow plus verbatim body leftovers),
  so only the narrowed residue re-diffs mixed by construction (WI-5d
  `single_block` still fuses pure let runs only); `14` divergent single-file renames whose
  deltas never share one intent — struct/enum/const/mod/nested-`Helper`/
  self-ty-`impl`/trait/trait-side-`impl` re-infer as WI-19c precision
  singles (8 semantic + 1 ordinary under partial partition) while the
  `impl Printable for Old` → `impl Labeled for New` double-delta pair
  raises with its trait+self-ty defs as one joint intent (WI-3,
  `item_joint_impl`, diffrs fixture `128-joint-impl-rename-fires`); all-single-delta
  multi-rename sections compress pair by pair instead (diffrs fixture
   `98-partition-fires`, canonically lossless)), `30` trait-side-only decl/def
   pairs (ordinary by design — no impl side shares the intent; mixed
   trait+impl classes raise via `trait_method_rename`, WI-4b); `06`
   standalone single-line insertions (ordinary by design — the
   rule-tag overhead exceeds native with zero template benefit;
   insertion joints with a shared rule ride WI-8/WI-13); zero-edit optional (`05`) re-diffs to empty,
which `canonicalize` rejects as unparseable. These are the only
non-tree-canonical cycles; every other corpus case is
canonically lossless, and one further iteration is a byte-identical
fixed point (`S″ == S′`) within a single diff mode.

Cross-mode section order: pending mode appends untracked new-file
sections after tracked ones while staged/rev flows are git-sorted
(probed `38-new-file-move`), so raw byte-identity (`S″ == S′`) is
only defined within one mode — pending/pending or staged/staged.
Canonical form sorts files and is the cross-mode comparator: the
same pre/post trees yield byte-identical canonical patches
regardless of the mode that produced each side.

---
