# RSmPL-Git Specification

**Version:** 0.1-draft  
**Status:** Experimental proposal  
**Primary audience:** implementers of LLM-centric patch generation, semantic refactoring engines, code-review tooling, and Rust-aware patch application systems  
**File extension suggestion:** `.rsmpl` 
**Canonical document name:** `specification.md`

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
10.4; `# lang=rust` is the marker because a bare rename carries no other
directive, Section 7.1):

```diff
diff --git src/lib.rs
@@ -10,7 @@ rule=rename_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }
```

A normal Git hunk remains normal:

```diff
@@ -1,4 +1,5 @@
 mod old;
+mod new;

 fn main() {}
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
compatibility with Git tooling. The two stripped paths must be identical:
differing real paths are a file rename, which v0.1 rejects with
`RSMPL_PARSE_CONFLICT` (Section 25.2) — run `git mv` first, then write one
file section per path with matching a/b sides (single-path form
preferred). `/dev/null` halves stay the create/delete spelling
(`diff --git /dev/null b/new`, `diff --git a/old /dev/null`).

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
```

The file contains:

1. one RSmPL hunk;
2. one ordinary Git hunk.

The `---` / `+++` / `index` lines above are optional file metadata
(`git_metadata*` in the grammar); the parser only requires the
`diff --git` line before the hunks. An empty patch (zero file
sections) is rejected with `RSMPL_PARSE_CONFLICT`: an empty
`--file`/stdin is almost certainly an authoring typo,
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
2. immediately after the hunk header, there is at least one unprefixed directive line beginning with `#`.

The tool is Rust-only, so the language defaults to Rust. A
`# lang=rust` directive may be included for explicitness but is not
required; `# lang=<other>` is rejected.

Because detection needs at least one `#` line, a hunk whose shape
requires no other directive (such as a bare rename) still writes one
marker line:

```diff
@@ -10,7 @@ rule=rename_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }
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

```diff
@@ -40,8 @@ rule=rename_calls
- foo(E)
# lang=rust
+ bar(E)
```

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

```diff
@@ -old_start,old_len +new_start,new_len @@ rule=name
```

```diff
@@ -old_start,old_len @@ rule=name
```

The elided form defaults the new side to the old side and is
RSmPL-only: ordinary hunks (no `#` directives) must keep both ranges,
because Git semantics need them. An elided new side on an ordinary
hunk is rejected with `RSMPL_PARSE_CONFLICT`.

Example:

```diff
@@ -10,7 @@ rule=rename_foo
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
```

The rule name is used for diagnostics, captures, dependency tracking, and review display. It is RECOMMENDED for multi-hunk patches but OPTIONAL for single-hunk patches; a hunk without `rule=` is valid and diagnostics show `<unnamed>` for it.

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

Most RSmPL-specific configuration should be placed in directive lines, not the hunk header.

Preferred:

```diff
@@ -40,8 @@ rule=rename_calls
# allow=many
# expr E
```

Not preferred:

```diff
@@ -40,8 +40,8 @@ rule=rename_calls lang=rust allow=many expr=E
```

The hunk header should stay close to normal Git style.

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
at least one `#` directive line, a hunk whose shape requires no other
directive (such as a bare rename) keeps `# lang=rust` as its marker;
hunks that already carry other directives omit it.

### 9.3 Core directives

| Directive | Meaning |
|---|---|
| `# lang=rust` | Accepted for explicitness; optional — hunks default to Rust, but at least one `#` line is required for detection, so directive-free hunks keep this line as their marker (Section 7.1). `# lang=<other>` is rejected. |
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
| `# bind=Name` | Capture exactly one rewritten or removed AST object under a cross-hunk name. Scalar only in this version. |
| `# use=Name` | Use a previously captured scalar object. |
| `# attrs=preserve` | Ignore attached attributes for match eligibility and preserve them across rewrite, deletion capture, and movement. Default. |
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

Multiple metavariables of the same kind may be declared in separate lines:

```text
# expr Receiver
# expr Arg
```

or in a comma-separated form if supported by the implementation:

```text
# expr Receiver, Arg
```

The separate-line form is canonical.

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

The engine must verify that `Old` resolves to `crate::old_api` before rewriting it. In v0.1 there is no semantic environment, so every symbol hunk needs `# fallback=syntax` (exact full-path text match, Section 19.3); without it the hunk fails with `RSMPL_SYMBOL_CONFLICT`.

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

> **v0.1 implementation note:** the anchor `-N,0` inserts *after* line `N` (at the start of line `N+1`, or EOF). `-0,0` inserts at the start of the file. Deleted-item cut ranges additionally absorb one adjacent blank separator line (following blank preferred, else preceding at EOF) so moves do not leave stray blank lines; the captured text itself excludes the separator.

### 10.6 Interaction with ordinary Git hunk coordinates

Ordinary Git hunks retain normal unified-diff semantics.

However, when an RSmPL-aware engine processes a mixed patch, it should plan all edits for a file against the original source coordinate system before materializing the final file. This gives both ordinary and semantic hunks a stable common base for conflict detection.

If an ordinary hunk and an RSmPL hunk edit overlapping original source ranges, the engine must either:

1. prove that the edits are compatible and merge them deterministically; or
2. reject the patch with an overlap conflict.

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

---

## 11. Ordinary Git Hunks

A hunk without a directive block is ordinary.

Example:

```diff
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,4 +1,5 @@
 mod old;
+mod new;

 fn main() {}
```

An RSmPL-Git engine should apply ordinary hunks using standard unified-diff semantics or delegate them to a Git-compatible patch applier, with one deliberate divergence: the header counts need not equal the body line counts. The engine positions by `old_start` and verifies every context (` `) and removal (`-`) line byte-for-byte against the original file; an arithmetic mismatch between the header counts and the body (` `+`-` vs ` `+`+`) alone never fails. `old_len` still bounds the file (`old_start-1+old_len` must fit, else `RSMPL_RANGE_CONFLICT`); `new_len` is fully ignored. A content mismatch (wrong `old_start` or wrong line text) fails with `GIT_HUNK_CONFLICT`. Exact line counting is not required — correct `old_start` plus correct body content suffices.

Ordinary hunks may appear before, between, or after RSmPL hunks.

A file section touched only by ordinary hunks is language-agnostic
(T137): the engine performs no Rust parse of the original file and no
Rust reparse of the rewritten file, so Markdown, configs, and any other
plain text flow through the same tool. Any section whose file carries
>=1 RSmPL hunk requires the target to parse as Rust (Section 25.2).

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

> **v0.1 implementation note:** root inference is heuristic. A single `fn`-shaped minus/plus pair is a function signature rewrite (rename and/or slot evolution); a bare `<kind> <Name>` pair for `struct`, `enum`, `trait`, `union`, `type`, `const`, `static`, `mod`, or `impl` is an item rename (Section 14.8); a bare `use <prefix>` pair is a use-path prefix rename (Section 14.9); a single bare declared metavariable (`- I`) is an item capture; an empty minus side is an insertion; `impl`-header context plus a function pair is an impl-method rename; `NAME!(...)` shapes are macro patterns; anything else call-like is an expression rewrite. A bare `impl Name` minus whose plus side is not a bare `impl New` pair is not an item rename: when both sides parse as Rust types the hunk falls through to the general type matcher (so `- impl Display` / `+ Box<dyn Display>` rewrites `impl Trait` bounds structurally); anything else stays a pattern error. `# root=item` forces item capture, `# root=expr` forces expression rewrite, `# root=stmt` forces a `let`-statement pattern, and `# root=type` forces a type pattern for ambiguous shapes. Non-call expression shapes, `let` statements, and type shapes are matched by the generalized matcher (see the v0.1 note in Section 18).

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
- fn foo<T>(...)
+ fn bar<T>(...)
  where
      T: Clone,
  {
      ...
  }
```

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

To bind and preserve arbitrary attributes explicitly:

```diff
@@ -10,8 @@ rule=rename_attributed_foo
# attributes A
- A
- fn foo(...) {
+ A
+ fn bar(...) {
      ...
  }
```

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
| `# attrs=preserve` | Default. Ignore attached attributes for match eligibility and preserve them across rewrite, deletion capture, and movement for any AST node kind that supports attributes. |
| `# attrs=absent` | Require no attached attributes or doc comments. |
| `# attrs=explicit` | Only attributes written in the pattern participate. Unwritten attached attributes cause the match to fail. |
| `# attrs=strip` | Match like `preserve`, but do not copy attributes to the replacement. This must be rejected unless the replacement target can legally omit the attributes. |

`# attrs=strip` is intentionally explicit because accidentally deleting documentation or attributes is high-risk.

Doc-comment mirrors (`///`, `//!`, and single-line `/** ... */`) normalize to `#[doc = "..."]` on any pattern path that accepts mirrored attributes; multi-line block docs cannot be mirrored — use an ordinary hunk.

### 14.7 Signature evolution

A function hunk whose `-` and `+` sides differ in more than the name evolves each explicitly written slot independently. Slots not written differently are preserved byte-for-byte (including the body, which is never rewritten). The reference implementation compares slots whitespace-insensitively against target text sliced from the original source:

| Slot | `-` matches target when | `+` effect when different from `-` |
|---|---|---|
| name | equal, or `$Meta` binds any | replaced with the `+` name (`$Meta` preserves) |
| visibility | absent requires inherited; concrete requires normalized equality; `V` matches any | insert/replace/delete `pub…` (`V` on `+` preserves) |
| qualifiers | same set (with `extern "ABI"`); `Q` matches any | insert/replace/delete qualifier run |
| generics | absent requires none; concrete requires normalized equality; `G` matches any | insert after name / replace `<…>` / delete |
| parameters | `...` matches any; concrete (incl. `()`) requires normalized equality | `...` preserves; concrete replaces the parenthesized list |
| return type | `...`-shorthand (no `->`) matches any; absent (via `# returns=none`) requires none; concrete requires normalized equality | `+` with no `->` preserves, except minus-concrete/`+`-omitted deletes `-> T`; concrete inserts after `)` or replaces `-> T` |
| where clause | absent requires none; concrete requires normalized equality of the predicates; `W` matches any | insert before `{` / replace / delete (minus-concrete/`+`-omitted) |
| body | must be `...` (or `;`) on both sides | always preserved; any concrete body is a pattern error |

Example (add visibility, generics, and a return type in one hunk, params and body preserved):

```diff
@@ -1,2 @@ rule=evolve
# lang=rust
- fn compute(...) {
+ pub async fn compute<T>(...) -> Vec<T> {
      ...
  }
```

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

Function renames (14.1–14.7) are strict about explicitly written slots because signatures evolve. Renaming any other item kind is rename-only: the author writes a bare `<kind> <Name>` pair and the engine replaces exactly one name-ident span per match, preserving visibility, generics, bodies, attributes, and formatting byte-for-byte (the same fast-path as a name-only function rename).

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

Rules:

- Both sides must name the same kind; a kind mismatch (`struct` → `enum`) is a pattern error. Identical minus/plus names are a pattern error.
- An optional leading `pub…` / `unsafe` / `mut` prefix is accepted for matching convenience, but the minus and plus spellings must agree (whitespace-insensitive): `struct Old` matches `pub struct Old<T> { ... }` and preserves everything except the ident, and `pub struct Old` / `pub struct New` renames likewise. Unlike function slots, there is no slot to evolve, so a *differing* prefix (`pub struct Old` / `struct New`) is a pattern error instead of a silent vis change. Prefix–kind legality is enforced: `unsafe` only on `trait`/`impl` headers, no visibility on `impl` (`unsafe struct`, `pub impl` are pattern errors). Any other concrete text (generics, fields, types, values, `where`) is a pattern error directing the author to the bare form.
- `$Meta` abstracted names are not supported for item renames in v0.1 (write concrete names). Backtick literal quoting follows Section 15.4.
- `auto trait` headers are not supported in v0.1: the author gets a fail-fast pattern error directing to an ordinary hunk (auto traits are never renamed in practice; silently falling through to a no-match would violate Section 2.6).
- `impl Old` renames the self-type ident and the trait-side path's last segment. It matches `impl Old`, `impl<T> Old<T>`, `impl Trait for Old`, and `impl Old for Foo`, rewriting only the `Old` spans while preserving generics, qualifiers, and the other side. Rationale: in valid Rust one name cannot be both a type and a trait in the same scope (type-namespace collision), so the bare pair is unambiguous; matching stays syntactic (Section 19). Generic (`impl Old<T> for Foo`), qualified (`impl crate::Old for Foo`), and negative (`impl !Old for Foo`) trait paths match on the last segment only.
- Matching reuses the function scope modes (`scope=contained/anchor/overlap/exact`, anchor on the ident line), attribute policies (`# attrs=...`), and `allow`/`optional` count validation. File-level, inline-`mod`-level, and block-level items are collected: the syntactic walk reaches items nested in fn bodies, closures, and method bodies, so a local `struct` renames like a top-level one (a top-level and a block-level same-named item need `# allow=many`, like any multi-match). Items inside macro bodies are not matched (Section 21). Each match emits one ident-span edit, so overlapping-match rules are trivially satisfied.

> **v0.1 implementation note:** `const fn` shapes keep routing to the function classifier (the keyword in name position falls through); `impl!(...)` macro shapes keep routing to the macro classifier (a `!` after the kind falls through). Item-rename hunks reject `# returns=none` (function-only). `auto trait` headers and raw-identifier edge cases beyond `` `r#name` `` are not supported; use an ordinary hunk.

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
  contain groups/globs/`as` tails, which are preserved.
- Matching is segment-wise on the target's leading path segments: minus
  `crate::old` matches `crate::old::Thing` but never `crate::old2::Thing`
  (never a string prefix). Identical minus/plus prefixes are a pattern error.
- Matching reuses the item scope modes (`scope=contained/anchor/overlap/
  exact`, anchor on the prefix-start line), attribute policies
  (`# attrs=...`), and `allow`/`optional` count validation. A file with
  several matching imports needs `# allow=many` (default `one` fails on
  the second match, like any multi-match hunk). Only file- and
  inline-`mod`-level `use` items are collected; `use` declarations nested
  inside function bodies are not matched in v0.1. Each match emits one
  prefix-span edit, so overlapping-match rules are trivially satisfied.
- No `# root=` is needed: a `use`-led minus/plus pair routes
  automatically (a `use_foo(E)` call keeps routing to the call matcher by
  the word-boundary rule). `# repeat` and `# returns=none` are rejected
  on `use` hunks (call-only and function-only, respectively).

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
| `pat` | pattern (whole-pattern placeholder, concrete pattern, or `P: T` parts in `let`) |
| `generic_params` | generic parameter list |
| `where_clause` | where clause |
| `visibility` | visibility marker such as absent, `pub`, `pub(crate)` |
| `qualifier` | one function qualifier |
| `qualifier_list` | zero or more function qualifiers |
| `attributes` | zero or more attributes or doc-comment-derived `#[doc = ...]` attributes (structural) |
| `block` | block expression or function body (structural) |
| `macro` | macro path/name |
| `token_tree` | macro token tree or token sequence |

Rejected in v0.1 with `RSMPL_PATTERN_CONFLICT`: `identifier`, `path`,
`lifetime`, `field`. These parse as declarations but have no kind-specific
engine binding — an agent authoring `# lifetime L` pays tokens for a
lifetime match that can never happen (Section 2.6). `stmt`, `attributes`,
`block`, `macro`, and `token_tree` are accepted as structural placeholders
(kind unchecked), not as distinct kind-checked binders.

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

The engine may reject lowercase metavariable names unless explicitly configured.

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

### 15.5 Optional sigiled metavariable mode

Implementations may support a stricter optional mode:

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

In this mode, unprefixed `E` always means a literal Rust identifier.

The default mode remains:

```diff
# metavars=bare
```

Bare mode is more token-efficient and is the default for LLM-generated patches. Sigiled mode is available for tools or authors that prefer zero shadowing ambiguity.

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

For `symbol` metavariables, repeated occurrences must resolve to the same semantic symbol. For `token_tree` metavariables in macro token-tree mode, repeated occurrences must contain the same balanced token-tree sequence after token-level trivia normalization.

In a `+` pattern, each occurrence of a bound metavariable inserts the same captured syntax. A `+` pattern may duplicate a captured value intentionally:

```diff
@@ -20,1 @@ rule=duplicate_arg
# expr E
- once(E)
+ twice(E, E)
```

If any repeated occurrence is inconsistent with the first binding, the candidate match is rejected rather than partially bound.


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
`foo(1, 2, 3)` to `bar(1, 2, 3)`. A `...` that absorbs nothing replays
empty and swallows one adjacent comma, so the same hunk rewrites
`foo(1)` to `bar(1)`. Omitting `...` on the `+` side is a deliberate
arg-drop (`- foo(E, ...)` / `+ bar(E)` rewrites `foo(1, 2)` to
`bar(1)`). A `...` in `+` with no matching `- ...` is a pattern error:
there is nothing to replay.

### 16.4 Body ellipsis

Pattern:

```rust
{
    ...
}
```

matches any block body, including an empty body.

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

#### `scope=anchor`

The first concrete non-ellipsis token in the pattern must be inside the old-side range.

Useful for matching a call inside a larger expression or statement.

> **v0.1 implementation note:** the anchor is approximated as the `fn` keyword line for function items and the candidate start line for call/macro matches.

#### `scope=overlap`

The matched AST node may overlap the old-side range.

Useful when the provided line range cuts through a larger node.

#### `scope=exact`

The matched AST node span must exactly equal the old-side range after trivia normalization.

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

### 17.6 Overlapping matches and `allow=many`

When `# allow=many` is set, matching is performed as a single top-down, source-order, non-overlapping pass.

The engine must:

1. walk candidate AST nodes in preorder, from outer nodes to inner nodes;
2. consider candidates in source order for nodes at the same depth;
3. accept the first candidate that matches and does not overlap an already accepted match;
4. skip all descendants of an accepted match for that hunk;
5. reject the hunk if two accepted candidates would partially overlap without one fully containing the other.

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

---

## 18. Rewrite Semantics

### 18.1 Replacement

A hunk with matching `-` and `+` patterns replaces the matched AST fragment.

> **v0.1 implementation note:** function-item hunks support signature evolution, not just renames. Each explicitly written slot evolves independently: an explicit `-`/`+` slot difference inserts, replaces, or deletes that slot's source bytes while all other slots (including the body and, unless `...` is replaced by concrete text, the parameter list) are preserved. Slot comparison is whitespace-insensitive against target text sliced from the original source; a concrete `-` slot must equal the target or the candidate is rejected (fail over guessing). `...` (parameters) and the same metavariable on both sides (visibility, qualifiers, generics, where) mean "preserve". A concrete body on either side is a compile error (bodies are always preserved). Minus-concrete/plus-omitted deletes the slot for parameters-as-`...`-removed, generics, and `where` clauses; for the return type, minus-concrete/plus-omitted deletes `-> T`, while plus-omitted with minus-`...` preserves the return type (rename fast-path). There is no syntax to delete a return type while matching any return with `...`; `# returns=none` constrains only the `-` side. Impl-block methods (`impl` context hunks) share the same slot semantics, including `self`/receiver parameters.
>
> Bare `<kind> <Name>` pairs for `struct`, `enum`, `trait`, `union`, `type`, `const`, `static`, `mod`, and `impl` (Section 14.8) are item renames, not general shapes: one ident-span edit per match, everything else preserved. `impl` renames the self-ty ident and the trait-side path's last segment.
>
> Non-call shapes are handled by the generalized matcher. Statements are limited to `let` bindings (`syn::Stmt::Local`, no `let-else`): the pattern may bind the whole pattern with one `# pat P` placeholder, use a concrete pattern, or spell an ascribed shape `P: T` with placeholder parts. A bare `P` never matches an annotated `x: T` (strict, like function slots); embedded metavariables elsewhere in the pattern are rejected at compile time. The initializer is matched with the general expression matcher.
>
> General expressions cover paths, literals, binary/unary/cast/reference/paren, calls, method calls, assignment, field access, indexing, arrays, tuples, struct literals, ranges, `await`, and `?`. General types cover paths with generic arguments, references, slices, arrays, tuples, parentheses, raw pointers (`*const`/`*mut`), fn-pointer types, `impl Trait`, `dyn Trait`, and `_` (which matches any type). Anything else (blocks, `if`/`match`/loops/closures, `let-else` targets, parenthesized path arguments such as `Fn(A) -> B` with placeholders, `...` wildcards in these shapes) rejects the candidate rather than guessing. Placeholder path segments may carry generic arguments (`P<u8>`, `P::<T>`) and method-call turbofish compares structurally (`collect::<T>` binds `T`); lifetimes are wildcards in v1 and are not compared. A call shape must consume the whole pattern: `foo(E) + 1` and `E.foo() + 1` route to the general matcher, not to a `foo(E)`/`E.foo()` call prefix (only a trailing `;` keeps call routing, so `foo(E);` stays a call). An empty `+` side deletes a `let` statement (whole lines) or a `...;` expression statement; type patterns cannot be deleted. Leading `#[...]` attributes are only supported on `let` patterns and follow `# attrs=...` (a `+` side that writes attributes the `-` side did not is rejected as ambiguous). The fail-fast reference is Section 18.6.

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

### 18.6 v0.1 general-matcher coverage table (fail-fast reference)

General `let`-statement, expression, and type patterns (Section 18.1, v0.1
note) cover a bounded shape set. Anything outside it fails fast: either at
compile time with the exact diagnostic below, or at match time as a silent
candidate rejection that surfaces as `RSMPL_MATCH_CONFLICT` with reason `no
match found` and a suggestion naming the file to read for valid ranges.
There is no partial matching and no guessing; check this table before
authoring instead of iterating against `--check`. The match-time (R-row)
rejections are permanent v0.1 scope, not deferred work: control-flow
expressions, `let-else` targets, `P @` subpatterns, and macro types go
through ordinary hunks (the same composition as Section 21.3 macro
bodies and Section 22 formatting). Reopen a row only with a dogfood
case where an ordinary hunk cannot express the edit.

#### Compile-time rejections (exact diagnostics)

| # | Trigger | Exact diagnostic |
|---|---|---|
| C1 | `...` anywhere in a general minus/plus side | `v1 general stmt/expr/type patterns do not support `...` (use a call pattern or a concrete shape)` |
| C2 | `let-else` in the minus pattern | `v1 general patterns do not support `let-else` statements` |
| C3 | `# root=stmt` with a non-`let` shape | `` `# root=stmt` requires a `let` statement pattern (v1 supports `let` only, no `let-else`) `` |
| C4 | `# root=expr` with a non-expression shape | `` `# root=expr` requires a Rust expression pattern `` |
| C5 | `# root=type` with a non-type shape | `` `# root=type` requires a Rust type pattern `` |
| C6 | empty `+` side on an expression | `v1 expression deletion (empty `+` side) is only supported for `...;` statement shapes; match a `let` statement or write an explicit replacement` |
| C7 | empty `+` side on a type | `v1 type patterns cannot be deleted (empty `+` side would produce invalid syntax); write an explicit replacement type` |
| C8 | embedded metavariable inside the `let` bound pattern (anything other than a whole-pattern `# pat P` placeholder, a concrete pattern, or `P: T` with placeholder parts) | `v1 `let` patterns support a whole-pattern `# pat P` placeholder, a concrete pattern, or `P: T` with placeholder parts; embedded metavariables elsewhere in the pattern are not supported` |
| C9 | `# returns=none` on a non-function hunk | `` `# returns=none` requires a function-item pattern `` |
| C10 | leading `#[...]` attributes on a non-`let` shape | `` leading attributes are only supported on `let` statement patterns in v1 `` |
| C11 | `+` side writes attributes the `-` side did not match | `replacement writes attributes the pattern did not match; write the same attributes in the `-` pattern or drop them` (suggestion: `` mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip` ``) |
| C12 | `+` attributes differ from matched `-` attributes | `replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges` (same suggestion as C11) |
| C13 | `# symbol` declared without `# fallback=syntax` in a call-shape hunk (no semantic environment in v0.1) | `` symbol `<Name>` could not be resolved (no semantic environment in v0.1) `` (code `RSMPL_SYMBOL_CONFLICT`, suggestion `` add `# fallback=syntax` to allow syntactic matching ``). Only call shapes reach this gate (general/macro shapes fail earlier at C14). |
 | C14 | `# symbol` in a general-expr/`let`/type or macro hunk (with or without `# fallback=syntax`) | `` `# symbol` is only supported in call shapes in v0.1 (call-callee, call-argument, and method-receiver symbols match by exact full-path text, SPEC 19.3); this <shape> hunk cannot use `# symbol` — use a call-shape hunk or a concrete path pattern `` (code `RSMPL_PATTERN_CONFLICT`, suggestion `use a call-shape hunk or a concrete path pattern`). A `# symbol` naming the *method ident* of a method call is also rejected: the method slot compares bare idents, never seeded paths, so it could never match — `` `# symbol old` names the method ident; method idents already match exactly — remove that `# symbol` line `` (method idents match as written; symbols verify receiver/callee paths). |

C6 note: `...;` expression-statement deletions never reach the general
matcher — call-like `foo(E);` shapes route to the call matcher (Section 13.4)
— so in practice C6 fires for every empty-`+` expression hunk. Delete a
`let` statement instead (whole lines), or write an explicit replacement.

Routing note: `let`-led patterns always reach the `let` gate — a
`let`-keyword callee guard stops the call router claiming `let (E, x)`
as a `let(...)` call (likewise `let [E, x]` and `let x = foo(E)`).
`let (E, x) = ...` therefore fails at gate C1 (`...` rejected) and
`let (E, x) = E;` at gate C8 (embedded metavariables), not with a bogus
`no match found` from the call matcher. Callees containing whitespace or
a statement keyword (`if`/`match`/`return`/...) likewise fall through to
the general matcher. A call shape must additionally consume the whole
pattern (balanced parens, then only whitespace or a lone `;`): `foo(E) + 1`
and `E.foo() + 1` route to the general matcher rather than a `foo(E)` /
`E.foo()` call prefix, while `foo(E);` keeps call routing.

#### Match-time silent rejections (surface as `no match found`)

| # | Trigger | Mechanism | Write instead |
|---|---|---|---|
| R1 | block, `if`, `match`, loops, or closures as the general expression | bounded variant set: paths, literals, binary/unary/cast/reference/paren, calls, method calls, assignment, field access, indexing, arrays, tuples, struct literals, ranges, `await`, `?`; anything else rejects the candidate | an ordinary Git hunk, or a call/expr shape around the inner expression |
| R2 | `let-else` in the *target* file | v1 never matches `let-else` targets (`diverge.is_some()` rejects the candidate) | an ordinary Git hunk |
| R3 | (closed by T144/T145) placeholder path segments with generic arguments match structurally (`P<u8>`, `P::<T>` bind the segment ident and compare args via `match_generic_arg`); method-call turbofish compares structurally too (lifetimes wildcard, `_` matches any type). Function-call turbofish compares structurally as well (`id::<T>` binds `T` to the target source text; concrete args compare token-normalized; the pattern must use the `::<>` form — a bare `id<T>` pattern never matches a real call) | parenthesized path arguments (`Fn(A) -> B` shapes with placeholders) still reject via rendered-text comparison | — |
| R4 | `P @ ...` subpattern bindings in `let` | `subpat.is_some()` rejects the candidate | a whole-pattern `# pat P` placeholder or a concrete pattern |
| R5 | unsupported type variants (macro types) | supported: paths with generic arguments, references, slices, arrays, tuples, parentheses, raw pointers (`*const`/`*mut`, constness strict), fn-pointer types (arity/unsafety/ABI strict, variadics never match), `impl Trait`, `dyn Trait` (bound count strict, `dyn` presence strict, `?` modifiers and `for<'a>` binders strict, plain lifetime bounds wildcard), `_` (which matches any type), `!`; anything else rejects the candidate | a concrete type or a whole-type `# type T` placeholder |

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

Symbol resolution may require:

- Cargo workspace information;
- module graph;
- dependencies;
- feature flags;
- macro expansion;
- Rust edition;
- `cfg` configuration.

If the semantic environment is unavailable, the engine must fail unless the hunk explicitly allows fallback.

Fallback directive:

```text
# fallback=syntax
```

In v0.1 `fallback=syntax` is the only supported path: there is no semantic environment, so the engine always matches syntactically and rejects symbol hunks without the fallback line. Matching is exact full-path text equality — a bare `old_api(...)` call site does not match `# symbol Old=crate::old_api`. For plain renames prefer a concrete path pattern (`- crate::old_api(E)` / `+ crate::new_api(E)`), which matches identically with fewer directives. `# symbol` in any other shape is rejected at C14 (SPEC 18.6).

> **v0.1 implementation note:** syntactic fallback compares the full path text exactly (e.g. `crate::old_api` matches only callees spelled `crate::old_api` — not bare `old_api`, not `other::old_api`). Symbol hunks without `# fallback=syntax` are rejected because v0.1 has no semantic environment. No resolver integration (rust-analyzer / cargo metadata) is planned for v0.1; revisit only with a dogfood case where exact-path matching blocks a real rename.

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

### 20.3 Capture naming and forward references

The value of `# bind=Helper` creates a named capture.

The value of `# use=Helper` imports that capture.

Names are patch-local.

In this version, `# bind=<Name>` is a scalar capture. It binds exactly one AST object. Collection-valued captures are not part of v0.1 and there is no syntax such as `# bind_list=Name`, `# bind_many=Name`, or `# use_list=Name`.

> **v0.1 implementation note:** `# bind=` is additionally restricted to item-capture cut hunks (a bare `- I` pattern), and `# use=` to pure insertion hunks (`+ Name`). Rewriting hunks (rename, call rewrite) cannot define captures, and non-insertion hunks cannot consume them. Captured text is the exact matched item span; paste validity at the insertion site is enforced by re-parsing the rewritten file.

A hunk that declares both `# bind=<Name>` and `# allow=many` is invalid in v0.1 unless a future version defines explicit collection-capture syntax. Authors who need to move several items must use separate scalar captures with distinct names.


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

- every `# use=<Name>` has exactly one corresponding scalar `# bind=<Name>`;
- no hunk combines `# bind=<Name>` with `# allow=many` in v0.1;
- forward references are resolved before application, not treated as missing captures;
- the cut hunk matched exactly one item;
- the pasted item is syntactically valid at the insertion site;
- a moved item is not simultaneously deleted, replaced, or moved by another incompatible hunk;
- imports, visibility, and module paths are not silently fixed unless explicitly requested.

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

In token-tree mode the body must be a bare `...` (verbatim
preservation); anything else is rejected with `RSMPL_PATTERN_CONFLICT`.
This is less structured than AST mode and should be used cautiously.

Composition (permanent answer, T133): invocation mode already binds
metavariables where the body is well-defined — comma-separated args
(`- assert_eq!(E, E)` / `+ assert_equal_once(E)`, with repeated-metavar
equality) and the lone whole-body capture (`- M!(TT)` / `+ new_macro!(TT)`
with `# macro M` + `# token_tree TT`). `# macro=token_tree` stays
verbatim-only (`NAME!(...)`, body preserved) by design: token-tree bodies
are token soup, not Rust lists (§21.4), so there is no delimiter contract
for binding `# expr`-style metavars inside them. Anything else nested in
a macro body goes through ordinary hunks — the same one-tool composition
as §14.9 (no new flags or directives; extending the macro matcher with a
token-level expression splitter is rejected, not deferred).

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

> **v0.1 implementation note:** multiple `...` wildcards (argument lists, path segments, macro bodies) match with a documented left-to-right greedy strategy; `foo(...)` absorbs any argument count including zero.

### 21.5 Expanded macro rewrite

```text
# macro=expanded
```

This mode requires the engine to know macro expansion output and map expanded spans back to source spans.

If the engine cannot provide that mapping, it must fail.

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

Document order may be used as a deterministic tie-breaker only for non-overlapping insertions at exactly the same original insertion point. Implementations should document their tie-breaking policy.

> **v0.1 implementation note:** multiple insertions at exactly the same point are applied in a deterministic (stable, document-order-derived) sequence; an insertion strictly inside a deleted or replaced range is rejected as an overlap conflict.

---

## 24. Validation Rules

An implementation must reject an RSmPL hunk if any required validation fails.

### 24.1 Structural validation

Reject if:

1. hunk header cannot be parsed;
2. a `# lang=` directive names a language other than `rust`;
3. directive syntax is invalid;
4. metavariable declarations are malformed;
5. literal-token quoting is malformed, including invalid raw-identifier quoting such as `` r#`type` `` instead of `` `r#type` ``;
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
4. a scalar `# bind` hunk matches more than one AST object or is combined with `# allow=many`;
5. a rewrite would produce duplicate or invalid syntax;
6. a move would remove a required node and fail to insert it.
7. a `# repeat` hunk is not a call/method-call/general-expression rewrite, is combined with `# bind=`/`# use=`, or has an empty minus/plus side.

### 24.5 Ordinary Git validation

Ordinary hunks must satisfy normal Git patch application rules.

---

## 25. Conflict and Error Model

The engine should report structured errors.

### 25.1 Range conflict

The declared range is invalid.

```text
RSMPL_RANGE_CONFLICT
rule: rename_calls
file: src/lib.rs
old_range: 40..47
reason: range exceeds file length
suggestion: read `src/lib.rs` to list valid ranges
```

The `suggestion` names the file to read (repo-relative path) so an
agent repairs without guessing flags.

### 25.2 Parse conflict

Rust parsing failed.

```text
RSMPL_PARSE_CONFLICT
rule: rename_foo
file: src/lib.rs
reason: target file does not parse as Rust
```

The Rust-parse gate fires only for files carrying >=1 RSmPL hunk
(Section 11): ordinary-only sections skip both the original parse and
the rewritten-file reparse, so non-Rust targets apply cleanly.

A `diff --git a/old b/new` section whose stripped paths differ is also a
parse conflict (a file rename, rejected fail-fast per Section 2.6 — v0.1
never silently writes to the b-side):

```text
RSMPL_PARSE_CONFLICT
file: src/new.rs
reason: file rename a/src/old.rs -> b/src/new.rs is not supported in v0.1
suggestion: use `git mv` first, then one file section per path with matching a/b sides (see SPEC 6)
```

### 25.3 Pattern conflict

The RSmPL pattern failed to parse. Unknown directives (`# foo`,
`# foo=bar`), `# edition=*`, and `# format_scope=*` are rejected here
(no silent ignore, Section 9.6), as is any hunk-header `k=v` tag other
than `rule=` (no silent dead tags, Section 8.3), any `# root=` value other
than `item/expr/stmt/type` (no silent heuristic fallthrough, Section 13.4),
any `# identifier/# path/# lifetime/# field` metavariable declaration
(no kind-specific binding in v0.1, Section 15.1), any `# repeat=<value>`
(write bare `# repeat`, Section 17.8), any `# repeat` on a
non-call/non-general-expression shape, on a capture hunk, or with an empty
minus/plus side (no silent no-op repeat, Sections 17.8, 24.4), any
item-rename hunk whose minus/plus `pub…` / `unsafe` / `mut` prefixes
differ (rename-only, no silent vis change — reason carries
`prefixes differ`), and any item-rename side with a kind-illegal prefix
(`unsafe` off `trait`/`impl`, visibility on `impl`), all Section 14.8,
and any call/method-call hunk whose `+` side holds more `...` tokens
than the `-` side has argument ellipses to replay (nothing to replay —
drop the `+ ...` or add a `- ...`, Section 16.3).

```text
RSMPL_PATTERN_CONFLICT
rule: rename_foo
reason: invalid item pattern
```

### 25.4 Match conflict

No match or too many matches.

```text
RSMPL_MATCH_CONFLICT
rule: rename_calls
file: src/lib.rs
old_range: 40..47
expected: one
actual: three
suggestion: add "# allow=many" if multiple rewrites are intended
```

A zero-match failure names the file to read instead:

```text
RSMPL_MATCH_CONFLICT
rule: rename_calls
file: src/lib.rs
old_range: 40..47
reason: no match found
suggestion: read `src/lib.rs` to list valid ranges; check the pattern or add `# optional`
```

When the engine can prove the range (not the pattern) is wrong, it appends
a zero-discovery retry (T62 probe: the same matcher re-run whole-file,
count-only). Exactly one match elsewhere appends `— exactly one match
exists elsewhere in the file; retry with `# scope=file` + `@@ -1,1 @@`;
several matches elsewhere append `— multiple matches
exist outside this range; widen the range (or add `# scope=file`) and add
`# allow=many` if intended`. (Counts saturate: matchers stop counting at 2
under `# allow=one`, so the many-case never claims an exact number.)

The zero-match `reason` also carries a T67 candidate preview — the most
similar same-kind item header (`fn foo at line 5: 'fn foo() {}'`), or the
nearest non-empty source line when the hunk kind has no item headers — so the next
attempt often needs no file read. The header walk reaches block-level items
(fn bodies, closures, method bodies — the same scope matching reaches), and
same-kind headers win over a nearer enclosing item (a typo'd `struct Lcoal`
previews the real `struct Local`, not the enclosing `fn`). Within the
same-kind set, name similarity (edit distance) ranks before line distance:
a typo'd `struct Betta` previews the real `struct Beta` one line further
down, not the nearer `struct Alpha`. The preview is a single line of at most
200 chars of source (plus a short `kind name at line N` label, whole string
bounded); `suggestion` is unchanged. An empty file has nothing to preview
and keeps the bare `no match found` reason. If the preview shows a
`let-else` target, a block / `if` / `match` / loop / closure expression,
a `P @` subpattern, or a macro type, the shape itself is outside v0.1
scope (Section 18.6 rows R1 / R2 / R4 / R5) — rewrite it with an
ordinary hunk instead of re-shaping the pattern.

### 25.5 Symbol conflict

Symbol resolution failed.

```text
RSMPL_SYMBOL_CONFLICT
rule: rename_symbol_calls
symbol: crate::old_api
reason: symbol could not be resolved under current Cargo configuration
```

### 25.6 Capture conflict

A capture is missing or invalid.

```text
RSMPL_CAPTURE_CONFLICT
rule: paste_helper
capture: Helper
reason: no previous hunk bound this capture
```

### 25.7 Git hunk conflict

A normal Git hunk failed.

```text
GIT_HUNK_CONFLICT
file: src/lib.rs
hunk: @@ -1,4 +1,5 @@
```

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
```

Patch `target_path` values that are absolute or escape the worktree
root fail here too (see 27.6) — before any file is read. Differing
`diff --git a/old b/new` paths (a file rename, Section 6) fail as a
parse conflict (25.2), not an I/O conflict: the paths are well-formed,
the operation is unsupported.

---

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
@@ -10,7 +10,7 @@
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
argument, not patch content. A missing target file warns
(`treating as new (empty) file`) and is treated as empty, so a
typo'd path creates a stray file on apply — read the warning before
committing; `--format json` consumers must also read stderr, where
all `warning:` lines live (see D.2.3). Created files lower with
`new file mode 100644` + `--- /dev/null` headers so the concrete
diff applies with a bare `git apply`. `--output <PATH>` is resolved
against the caller cwd, never joined to `--repo`: it is the agent's
local artifact, not repo content. File renames are not a containment
issue: differing `diff --git` paths are rejected earlier as a parse
conflict (Sections 6, 25.2).

---

## 28. Recommended LLM Emission Style

### 28.1 Use ordinary Git hunks for simple line edits

Good:

```diff
@@ -1,3 +1,4 @@
 mod a;
+mod b;
```

Do not use RSmPL for trivial additions unless semantic placement is important.

### 28.2 Use RSmPL for semantic renames

Good (diet form: elided new side, no `# lang=rust` next to other
directives — Sections 7.1, 10.4):

```diff
@@ -40,8 @@ rule=rename_calls
# allow=many
# expr E
- old_api(E)
+ new_api(E)
```

### 28.3 Use ranges aggressively

Keep RSmPL ranges tight: cite the smallest range that contains the match
(read the file for exact spans). When the agent already holds
a small file, skip range discovery entirely with whole-file scope:

```diff
diff --git src/lib.rs
@@ -1,1 @@ rule=rename_one_block
# scope=file
# allow=many
# expr E
- old_api(E)
+ new_api(E)
```

The `@@ -1,1 @@` range is a dummy (SPEC 17.3) — no line counting, no
discovery roundtrip.

Better (tight range):

```diff
@@ -80,15 @@ rule=rename_one_block
```

Worse (whole-file range: forces the author to count the file length, and
the range goes stale on every edit):

```diff
@@ -1,500 @@ rule=rename_whole_file
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

```diff
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
# item I
# bind=Moved
- I
```

and later:

```diff
# use=Moved
+ Moved
```

---

## 29. Examples

All examples use the diet-minimal recipe from Section 6 (single-path
header, elided new side, no `# lang=rust` next to other directives —
the form agents should emit). Bare renames keep `# lang=rust` as the
sole-directive marker (Section 7.1).

### 29.1 Rename a plain function

```diff
diff --git src/lib.rs
@@ -10,7 @@ rule=rename_plain_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }
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

```diff
diff --git src/lib.rs
@@ -30,12 @@ rule=rename_complex_foo
# lang=rust
- pub async unsafe fn foo<T>(...)
+ pub async unsafe fn bar<T>(...)
  where
      T: Clone,
  {
      ...
  }
```

This only matches functions with the explicitly written shape.

### 29.3 Rename calls in a known range

```diff
diff --git src/lib.rs
@@ -40,8 @@ rule=rename_calls
# allow=many
# expr E
- foo(E)
+ bar(E)
```

### 29.4 Method call to free function

```diff
diff --git src/service.rs
@@ -75,20 @@ rule=method_to_free_function
# allow=many
# expr Receiver
# expr Arg
- Receiver.old_method(Arg)
+ new_function(Receiver, Arg)
```

### 29.5 Semantic rename with symbol resolution

```diff
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
```

### 29.7 Mixed RSmPL and ordinary Git patch

```diff
diff --git src/lib.rs
@@ -10,7 @@ rule=rename_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }

@@ -40,8 @@ rule=rename_calls
# allow=many
# expr E
- foo(E)
+ bar(E)

@@ -1,4 +1,5 @@
 mod old;
+mod new;

 fn main() {}
```

### 29.8 Insert a use item semantically

```diff
diff --git src/lib.rs
@@ -3,0 @@ rule=insert_use
# lang=rust
+ use crate::new_module::Thing;
```

### 29.9 Rewrite inside an impl block

```diff
diff --git src/service.rs
@@ -20,40 @@ rule=rename_impl_method
# lang=rust
  impl Service {
      ...
-     fn old_name(...) {
+     fn new_name(...) {
          ...
      }
  }
```

### 29.10 Optional migration

```diff
diff --git src/lib.rs
@@ -100,30 @@ rule=optional_old_api_cleanup
# optional
# allow=many
# expr E
- old_api(E)
+ new_api(E)
```

If no calls exist in the range, the hunk succeeds without changes.

### 29.11 Stable Line Ranges Independent of Prior Hunks

This example demonstrates the original-source coordinate system from Section 10.2. The second semantic hunk keeps using the line number it had in the original file, even though the first hunk inserts a larger replacement before it.

```diff
diff --git src/lib.rs
@@ -10,1 @@ rule=insert_large_block
# lang=rust
- old_function()
+ new_function_that_spans(
+     many,
+     many,
+     lines,
+ )

@@ -20,5 @@ rule=rename_second_function
# lang=rust
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
                      | "attributes"
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
`git` must be on `PATH`; without it that step fails with
`` failed to run `git apply` (is git installed?) `` under
`GIT_HUNK_CONFLICT`. Matching, lowering, and `--output` are
pure and never spawn git.

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
8. define collection-valued capture syntax in v0.1;
9. define a complete Rust parser grammar.

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
# rsmpl-git=0.1
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
- explicit collection-valued captures and list paste separators;
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
`# lang=rust` is the marker only for otherwise directive-free hunks (bare
renames) and is omitted when other directives are present (Sections 7.1,
9.2). The verbose compatible spelling used in the Section 29 examples
(two-path headers, copied new sides, explicit `# lang=rust`) stays valid.

The canonical ordinary hunk shape remains:

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

The patch format above is engine-agnostic, but the reference `rsmpl-git`
binary exposes agent-oriented conveniences. They do not change patch
semantics; they reduce the tokens and round-trips an LLM needs to author
valid `@@ -old,len` ranges and verify results. The tool is patching-only:
range discovery is a targeted file read (the agent's own tooling) plus
whole-file scope (D.3), not a CLI listing mode.

### D.1 Piped patches (stdin)

No `--file` flag means stdin (Unix filter default), so the cheapest
spelling needs no input flag at all:

```sh
# Run from the repo root; from elsewhere, add --repo ./my-repo to any command:
cat change.rsmpl | rsmpl-git --check
```

`--file -` spells the same stdin read explicitly. Empty stdin is rejected
with the T59 `RSMPL_PARSE_CONFLICT` (empty input is an authoring typo,
never silent success). A bare interactive call with no pipe blocks on
stdin like any Unix filter — pipe or pass `--file`.

Two sources cover every case: stdin keeps small patches temp-file-free
while `--file` avoids shell-quoting for large patches (T135 removed the
redundant `--patch '<diff>'` inline spelling, which duplicated stdin with
worse quoting and arg-length failure modes). Canonical stdin spelling is
bare stdin (no flags); `--file -` is the explicit alias.

### D.2 Machine-readable reports (`--format json`)

`--format text` (default) preserves the existing human output. `--format json`
prints one JSON object to stdout:

```json
{
  "version": 1,
  "rules": [{"rule": "rename_foo", "file": "src/lib.rs", "matches": 1, "spans": [[10, 12]]}],
  "files": [{"path": "src/lib.rs", "added": 1, "removed": 1}],
  "diff": "diff --git ... (lowered concrete patch, no RSmPL directives)",
  "diff_truncated": false,
  "diff_lines": 12,
  "diff_shown": 12,
  "check": true
}
```

- `rules` is the per-rule match report (text mode prints only one-line
  `applied`/`check` summaries; agents needing match counts use JSON).
  Each entry carries `spans`: one original-source 1-based inclusive
  `[start, end]` line range per match in application order, so
  `spans.len() == matches` always holds and agents can verify what matched
  without re-reading the file. Insertions report the anchor as `[L, L]`;
  an empty match set (optional no-match) reports `[]`.
- `files` lists per-file `{path, added, removed}` line stats computed from
  the full old/new contents, so counts stay exact even when `diff` is
  truncated or omitted (see D.2.1). Empty when no changes. `files[].path`
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
- `version` is the JSON report schema version (`1` = this contract:
  boolean `check`, no `files_changed`). Agents pin parsing on it; any
  future breaking JSON change bumps it.
- `check` is a boolean: `true` when `--check` was passed (the lowering,
  empty or not, passed `git apply --check`), else `false`. Failures
  never reach this payload (exit 2 with the `{error}` envelope below),
  so `false` means "skipped", never "failed". The single-type shape
  replaces the former `"ok"`/`null` pair, which forced agents to branch
  on type.
- `rules` follows patch document order (file-section order, then
  hunk order within a section), so a multi-hunk patch reads
  top-to-bottom like the patch itself — including move cut/paste
  hunks, which the engine evaluates in a different pass but
  reports in document position.
- `files` follows patch file-section order.
- `--output <path>` writes the full diff file in both modes, even when
  the reported `diff` is truncated or omitted — but writes nothing when
  the lowering is empty (no changes, no file). It is written before
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
- Defaults audit: `--format text` and `--repo .` stay; the lowering is
  always 0-context (T128: the former `--context 3` default is gone — minimal
  tokens by default, still appliable via the `--unidiff-zero` wrapper,
  D.2.2). Text keeps human review working (JSON is a ~4-token opt-in);
  `--repo` defaults to `.`, so run from the worktree root and omit it,
  adding `--repo DIR` only when invoking from elsewhere.
- Failure envelope: with `--format json`, every patch/IO failure prints
  one JSON envelope to stdout and exits 2, so an agent can repair without
  parsing stderr prose. The failure is stdout-only: no stderr `error:`
  line is printed in JSON mode (T89 symmetry with the silenced success
  `check:` line); stderr carries `warning:` lines only. Text mode keeps
  the stderr diagnostic.

```json
{"version": 1, "error": {"code": "RSMPL_MATCH_CONFLICT", "rule": "r", "file": "src/lib.rs", "range": "1..2", "reason": "no match found", "suggestion": "read `src/lib.rs` to list valid ranges; check the pattern or add `# optional`"}}

```

- The envelope always carries `version` plus all six error keys (`code
  rule file range reason suggestion`); absent optionals are `null`. `code` is one of the SPEC 25
  codes, including `RSMPL_IO_CONFLICT` (25.8) for unreadable patch, repo,
  or output files, stdin failures, and flag-usage errors.
- Flag-usage errors (unknown flag, bad value) are enveloped too: with
  `--format json` they print the same stdout-only `RSMPL_IO_CONFLICT`
  envelope (reason = clap's one-line error, suggestion = run
  `` `rsmpl-git --help` ``) and exit 2. Text mode keeps clap's stderr
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
rsmpl-git --file change.rsmpl --format json --diff-max-lines 0 --check

# First 40 lines of diff for a quick skim, stats stay exact:
rsmpl-git --file change.rsmpl --format json --diff-max-lines 40 --check

# Text equivalent (stdout is the reported diff, markers on stderr):
rsmpl-git --file change.rsmpl --check --diff-max-lines 40
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

#### D.2.2 Lowering context (always 0)

D.2.1 bounds the *reported* diff; the *lowered* diff itself is always
0-context (T128: the former `--context N` flag is removed — one fewer flag,
minimal tokens by default):

```sh
# Run from the repo root; from elsewhere, add --repo ./my-repo to any command:
# Minimal tokens by default, still a real appliable patch:
rsmpl-git --file change.rsmpl --check
```

Rules:

- The lowering shows only changed (`-`/`+`) lines; hunk headers stay
  `@@ -a,b +c,d @@`. rsmpl-git's own `--check`/apply/`--output` path handles
  it (it passes `git apply --unidiff-zero`, which plain `git apply`
  requires for zero-context hunks); piping a saved patch to a
  bare `git apply` needs that same flag.
- `--check`, apply, and `--output` all see the same 0-context
  lowering. A truncated report (`diff_truncated: true`) is review-only,
  but a 0-context diff is appliable. Bare-`git apply` compatibility is
  intentionally traded for token minimalism (agent-centric default).
- Per-file `{path, added, removed}` stats are computed
  from the full old/new contents, so they stay exact under any report budget.
- Layering: the D.2.1 report budget applies after lowering.
  `--diff-max-lines 2` shows the first 2 lines of the 0-context diff;
  `--diff-max-lines 0` omits with `diff_truncated: true` iff
  changes exist. With no changes the budget flag is a no-op.
- Three-lever summary: `--format` chooses the report language (text diff
  vs JSON object), `--diff-max-lines` trims only the reported `diff`
  preview, and `--output` always saves the full lowering —
  so `--format json --diff-max-lines 0 --check --output
  full.patch` verifies with zero preview tokens while keeping the
  appliable patch on disk.
- The removed `--context` spelling is a clap usage error (unknown
  argument): plain text by default, `RSMPL_IO_CONFLICT` envelope with
  `--format json` (usage errors are enveloped, D.2).

#### D.2.3 Output streams and empty lowerings

Agent loops pipe stdout (to `git apply`, to a JSON parser) and must know
exactly which bytes land on which stream. The contract, per mode:

| Mode | stdout | stderr |
|---|---|---|
| text apply | empty (quiet by default) | `applied N file(s): …` (kept: one line of human reassurance) |
| text `--check` | reported diff (empty when omitted/no changes) | budget markers + `check: lowered patch applies cleanly` |
| JSON apply | report object | `warning:` lines only (failures live in the stdout envelope, no stderr `error:` line) |
| JSON `--check` | report object with `"check":true` | `warning:` lines only (no `check:` line: stdout is the signal; failures likewise stdout-only) |

Rules:

- An empty lowering (no changes) never touches git: `git apply
  --check` is skipped and the run reports clean. Text prints
   `(no changes)` to stdout; JSON returns the report silently
   (`"diff": ""`, `"diff_truncated": false`, `"files": []`,
   `"check": true` when `--check` was passed, else `false`).
  `--output` writes nothing when the lowering is empty.
- `--output` is written before `--check` runs, so a failing `--check`
  still leaves the full appliable diff on disk for debugging.
- A truncated report (`--diff-max-lines N` cut lines off) is
  review-only: never pipe truncated stdout to `git apply`. Re-run
  without the flag or take the full diff from `--output`.
- `warning:` lines (`… treating as new (empty) file`, `skipping …:
  …`) go to stderr in both modes. A `--format json` consumer parsing
  stdout must also read stderr, or it will miss them.

#### D.2.4 Repo-free patch lint (no new flags)

Authoring check without a checkout: parse, directive validation,
pattern compilation, and the capture graph (`# bind=`/`# use=`,
plus-side binding) all run before matching, so an empty directory
stands in for the repo:

```sh
mkdir -p /tmp/empty && rsmpl-git --file change.rsmpl --repo /tmp/empty --check
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
pattern-compile/capture stages, so authored `@@ -old,len` ranges
mask those stages on the empty dir (the first range error wins). To
lint the deeper stages, temporarily lint with the `# scope=file` +
`@@ -1,1 @@` dummy (17.3) on each hunk — the dummy skips range
validation and the compile/capture errors surface. Expect the stderr
`treating as new (empty) file` warning (D.2.3); insertion-only
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

Agent workflow detail lives in `skills/rsmpl-git/SKILL.md`.

Recommended agent loop:

```text
read file -> author hunks in original-source coordinates
-> apply with --format json --diff-max-lines 0 --check -> review files[] stats
-> widen with --diff-max-lines N / --output full.patch only if needed
-> apply for real
```

New authors: start with `AGENTS.md` (one-page authoring reference); this
appendix and Sections 9/18.6 are the normative detail behind it.

---
