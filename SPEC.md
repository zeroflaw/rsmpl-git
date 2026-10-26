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

A minimal RSmPL hunk looks like this:

```diff
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,7 +10,7 @@ rule=rename_foo
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
@@ -10,7 +10,7 @@ rule=rename_foo
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
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -40,8 +40,8 @@ rule=rename_calls
# lang=rust
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
@@ -40,8 +40,8 @@ rule=rename_calls
# lang=rust
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
patch           ::= file_section*

file_section    ::= git_file_header file_body

git_file_header ::= "diff --git " old_path " " new_path newline
                    git_metadata*

file_body       ::= hunk*

hunk            ::= ordinary_hunk | rsmpl_hunk
```

Example:

```diff
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,7 +10,7 @@ rule=rename_foo
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
`diff --git` line before the hunks. The canonical agent recipe is
therefore the 1-line header:

```diff
diff --git a/src/lib.rs b/src/lib.rs
@@ -10,7 +10,7 @@
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }
```

No `---` / `+++` / `index` lines, no `rule=` tag (diagnostics then
show `<unnamed>`; add `rule=name` when a patch has several hunks).
For RSmPL hunks the `+new,len` side is ignored (Section 10.4), so
the recipe copies the old side verbatim (`@@ -10,7 +10,7 @@`).

---

## 7. RSmPL Hunk Detection

### 7.1 Detection rule

A hunk is an RSmPL hunk if and only if:

1. it has a normal unified-diff hunk header; and
2. immediately after the hunk header, there is at least one unprefixed directive line beginning with `#`; and
3. one of the directive lines declares:

```text
# lang=rust
```

Example:

```diff
@@ -40,8 +40,8 @@ rule=rename_calls
# lang=rust
# allow=many
# expr E
- foo(E)
+ bar(E)
```

This is an RSmPL hunk.

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
@@ -40,8 +40,8 @@ rule=rename_calls
# lang=rust
# expr E
- foo(E)
+ bar(E)
```

Invalid:

```diff
@@ -40,8 +40,8 @@ rule=rename_calls
- foo(E)
# lang=rust
+ bar(E)
```

### 7.4 Directive lines are not source lines

Directive lines are not part of the source file and are not counted against old or new hunk line counts.

In this hunk:

```diff
@@ -40,8 +40,8 @@ rule=rename_calls
# lang=rust
# allow=many
# expr E
- foo(E)
+ bar(E)
```

the three `#` lines are metadata. They do not represent Rust comments, removed lines, added lines, or context lines.

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

An RSmPL hunk uses a standard-looking unified-diff hunk header:

```diff
@@ -old_start,old_len +new_start,new_len @@ rule=name
```

Example:

```diff
@@ -10,7 +10,7 @@ rule=rename_foo
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
@@ -80,20 +80,20 @@ rule=rename_calls
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

Optional header tags:

```text
id=<identifier>
depends=<rule_name>
```

Most RSmPL-specific configuration should be placed in directive lines, not the hunk header.

Preferred:

```diff
@@ -40,8 +40,8 @@ rule=rename_calls
# lang=rust
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

### 9.2 Required directive

Every RSmPL hunk must include:

```text
# lang=rust
```

### 9.3 Core directives

| Directive | Meaning |
|---|---|
| `# lang=rust` | Marks the hunk as a Rust semantic patch hunk. Required. |
| `# allow=one` | Exactly one match is expected. Default. |
| `# allow=many` | Multiple matches may be rewritten. |
| `# optional` | Zero matches are allowed. |
| `# optional=true` | Equivalent to `# optional`. |
| `# optional=false` | Zero matches are an error. Default. |
| `# scope=contained` | Matched AST node must be fully inside the old range. Default. |
| `# scope=anchor` | First concrete non-ellipsis token must be inside the old range. |
| `# scope=overlap` | Matched AST node may overlap the old range. |
| `# scope=exact` | Matched AST node span must exactly match the old range after trivia normalization. |
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
| `# format=rustfmt` | Format affected output with rustfmt. |
| `# format=canonical` | Use engine canonical pretty-printer. |

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
@@ -80,20 +80,20 @@ rule=rename_symbol
# lang=rust
# allow=many
# symbol Old=crate::old_api
# symbol New=crate::new_api
- Old(...)
+ New(...)
```

The engine must verify that `Old` resolves to `crate::old_api` before rewriting it.

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

---

## 10. Line Range Semantics

### 10.1 Source of file and range

RSmPL-Git does not use separate `file:` and `lines:` fields.

The file is determined by the surrounding Git file section:

```diff
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
```

The range is determined by the hunk header:

```diff
@@ -40,8 +40,8 @@ rule=rename_calls
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

is interpreted against the **original file contents before any hunk in the patch file is applied**.

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

defines the region in the original source file where AST matching is permitted.

Example:

```diff
@@ -40,8 +40,8 @@ rule=rename_calls
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

Implementations should tolerate new-side line-count drift for RSmPL hunks and compute the final concrete line counts when lowering the RSmPL-Git patch to an ordinary Git diff. The new-side values are not consulted for RSmPL matching at all, so an agent that wants zero decision cost may copy the old side verbatim (e.g. `@@ -10,7 +10,7 @@`).

For example:

```diff
@@ -20,12 +20,0 @@ rule=cut_helper
# lang=rust
# item I
# bind=Helper
- I
```

contains only one removed pattern line, but the old-side extent is 12 original source lines.

### 10.5 Zero-length ranges and insertion anchors

Zero-length old-side ranges are allowed for insertions.

Example:

```diff
@@ -5,0 +5,0 @@ rule=paste_helper
# lang=rust
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

An RSmPL-Git engine should apply ordinary hunks using standard unified-diff semantics or delegate them to a Git-compatible patch applier.

Ordinary hunks may appear before, between, or after RSmPL hunks.

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
@@ -10,7 +10,7 @@ rule=rename_foo
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
@@ -20,10 +20,10 @@ rule=rewrite_inside_impl
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
@@ -100,50 +20,50 @@ rule=move_large_function
# lang=rust
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
@@ -40,8 +40,8 @@ rule=rename_calls
# lang=rust
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

> **v0.1 implementation note:** root inference is heuristic. A single `fn`-shaped minus/plus pair is a function signature rewrite (rename and/or slot evolution); a bare `<kind> <Name>` pair for `struct`, `enum`, `trait`, `union`, `type`, `const`, `static`, `mod`, or `impl` is an item rename (Section 14.8); a single bare declared metavariable (`- I`) is an item capture; an empty minus side is an insertion; `impl`-header context plus a function pair is an impl-method rename; `NAME!(...)` shapes are macro patterns; anything else call-like is an expression rewrite. `# root=item` forces item capture, `# root=expr` forces expression rewrite, `# root=stmt` forces a `let`-statement pattern, and `# root=type` forces a type pattern for ambiguous shapes. Non-call expression shapes, `let` statements, and type shapes are matched by the generalized matcher (see the v0.1 note in Section 18).

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
# root=pat
# root=impl_item
# root=trait_item
```

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
@@ -10,7 +10,7 @@ rule=rename_unit_foo
# lang=rust
# returns=none
- fn foo(...) {
+ fn bar(...) {
      ...
  }
```

### 14.3 Explicitly matching visibility

To match a public function, write `pub`:

```diff
@@ -10,7 +10,7 @@ rule=rename_pub_foo
# lang=rust
- pub fn foo(...) {
+ pub fn bar(...) {
      ...
  }
```

To match `pub(crate)`:

```diff
@@ -10,7 +10,7 @@ rule=rename_pub_crate_foo
# lang=rust
- pub(crate) fn foo(...) {
+ pub(crate) fn bar(...) {
      ...
  }
```

To abstract over visibility, declare a visibility metavariable:

```diff
@@ -10,7 +10,7 @@ rule=rename_any_visible_foo
# lang=rust
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
@@ -10,7 +10,7 @@ rule=rename_async_foo
# lang=rust
- async fn foo(...) {
+ async fn bar(...) {
      ...
  }
```

To match an unsafe function:

```diff
@@ -10,7 +10,7 @@ rule=rename_unsafe_foo
# lang=rust
- unsafe fn foo(...) {
+ unsafe fn bar(...) {
      ...
  }
```

To match an async unsafe function:

```diff
@@ -10,7 +10,7 @@ rule=rename_async_unsafe_foo
# lang=rust
- async unsafe fn foo(...) {
+ async unsafe fn bar(...) {
      ...
  }
```

To abstract over qualifiers:

```diff
@@ -10,7 +10,7 @@ rule=rename_qualified_foo
# lang=rust
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
@@ -20,10 +20,10 @@ rule=rename_generic_foo
# lang=rust
- fn foo<T>(...)
+ fn bar<T>(...)
  {
      ...
  }
```

To match a generic function with a where clause:

```diff
@@ -20,12 +20,12 @@ rule=rename_where_foo
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
@@ -20,12 +20,12 @@ rule=rename_any_generic_foo
# lang=rust
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
@@ -10,1 +10,1 @@ rule=rename_stmt_initializer
# lang=rust
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
@@ -10,8 +10,8 @@ rule=rename_documented_foo
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
@@ -20,6 +20,0 @@ rule=cut_helper
# lang=rust
# item I
# bind=Helper
- I
```

The binding `Helper` contains both the function item and its attached doc comments and attributes.

#### Explicitly constraining attributes

To require a specific attribute, write it explicitly:

```diff
@@ -10,8 +10,8 @@ rule=rename_inline_foo
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
@@ -10,8 +10,8 @@ rule=rename_attributed_foo
# lang=rust
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
@@ -10,7 +10,7 @@ rule=rename_unattributed_foo
# lang=rust
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
@@ -1,2 +1,2 @@ rule=evolve
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
@@ -1,1 +1,1 @@ rule=strip_result
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

> **v0.1 implementation note:** evolved slots are emitted with canonical spacing (`<T>`, `x: i32`, `where T: Clone`); only `# format=preserve` is implemented, so no `rustfmt` normalization pass runs. Deleting a slot absorbs one following space to avoid doubled spacing.

### 14.8 Other item renames

Function renames (14.1–14.7) are strict about explicitly written slots because signatures evolve. Renaming any other item kind is rename-only: the author writes a bare `<kind> <Name>` pair and the engine replaces exactly one name-ident span per match, preserving visibility, generics, bodies, attributes, and formatting byte-for-byte (the same fast-path as a name-only function rename).

Example (rename a struct without restating its fields):

```diff
@@ -1,3 +1,3 @@ rule=rename_struct
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
| `impl` | `impl Old` | `impl Old { ... }` (self-ty only, see below) |

Rules:

- Both sides must name the same kind; a kind mismatch (`struct` → `enum`) is a pattern error. Identical minus/plus names are a pattern error.
- An optional leading `pub…` / `unsafe` / `mut` prefix is accepted but ignored for matching: `struct Old` matches `pub struct Old<T> { ... }` and preserves everything except the ident. Unlike function slots, there is no slot to evolve, so requiring the author to restate visibility or generics would only cost tokens. Any other concrete text (generics, fields, types, values, `where`) is a pattern error directing the author to the bare form.
- `$Meta` abstracted names are not supported for item renames in v0.1 (write concrete names). Backtick literal quoting follows Section 15.4.
- `impl Old` renames the self-type ident only. It matches `impl Old`, `impl<T> Old<T>`, and `impl Trait for Old`, preserving generics and trait references. It does not match `impl Old for Foo` (where `Old` is the trait, not the self-ty); updating trait-side paths in impl headers is a path rename (a T19-class extension), not an item rename.
- Matching reuses the function scope modes (`scope=contained/anchor/overlap/exact`, anchor on the ident line), attribute policies (`# attrs=...`), and `allow`/`optional` count validation. Only file- and inline-`mod`-level items are collected; items nested inside function bodies are not matched in v0.1. Each match emits one ident-span edit, so overlapping-match rules are trivially satisfied.

> **v0.1 implementation note:** `const fn` shapes keep routing to the function classifier (the keyword in name position falls through); `impl!(...)` macro shapes keep routing to the macro classifier (a `!` after the kind falls through). Item-rename hunks reject `# returns=none` (function-only). `auto trait` headers and raw-identifier edge cases beyond `` `r#name` `` are not supported; use an ordinary hunk.

---

## 15. Metavariables

Metavariables bind Rust syntax or semantic objects.

RSmPL deliberately allows compact metavariable use such as `E`, `T`, and `I`, because that is token-efficient for LLM-generated patches. This creates a possible ambiguity when the source code itself contains an identifier with the same spelling as a declared metavariable. This section defines the resolution rule and the escape syntax.

### 15.1 Supported metavariable kinds

| Kind | Matches |
|---|---|
| `identifier` | Rust identifier |
| `path` | Rust path |
| `symbol` | resolved semantic symbol |
| `expr` | expression |
| `stmt` | statement |
| `item` | item, including its attached attribute/doc-comment envelope unless overridden by `# attrs=...` |
| `type` | type |
| `pat` | pattern |
| `lifetime` | lifetime |
| `generic_params` | generic parameter list |
| `where_clause` | where clause |
| `visibility` | visibility marker such as absent, `pub`, `pub(crate)` |
| `qualifier` | one function qualifier |
| `qualifier_list` | zero or more function qualifiers |
| `attributes` | zero or more attributes or doc-comment-derived `#[doc = ...]` attributes |
| `block` | block expression or function body |
| `field` | struct field |
| `macro` | macro path/name |
| `token_tree` | macro token tree or token sequence |

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
# identifier Name
```

### 15.4 Metavariable use and literal shadowing

A declared metavariable name shadows literal Rust syntax of the same spelling in compatible syntactic positions.

Example:

```diff
@@ -50,8 +50,8 @@ rule=box_type
# lang=rust
# type E
- Result<T, E>
+ Result<T, Box<E>>
```

In this hunk, `E` is a type metavariable. The pattern does **not** require a literal type parameter named `E`; it binds whatever type appears in that position.

If the pattern must match the literal Rust identifier `E`, the identifier must be escaped with RSmPL literal-token quoting. RSmPL uses backtick quoting for literal tokens whose spelling would otherwise be interpreted as a metavariable:

~~~diff
@@ -50,8 +50,8 @@ rule=literal_error_type
# lang=rust
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
@@ -50,8 +50,8 @@ rule=sigiled_example
# lang=rust
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
@@ -50,8 +50,8 @@ rule=wrap_expr
# lang=rust
# expr E
- old(E)
+ new(E)
```

Type metavariable:

```diff
@@ -50,8 +50,8 @@ rule=box_type
# lang=rust
# type T
- Vec<T>
+ Box<[T]>
```

Item metavariable:

```diff
@@ -20,12 +20,0 @@ rule=cut_item
# lang=rust
# item I
# bind=CutItem
- I
```

Pattern metavariable:

```diff
@@ -50,8 +50,8 @@ rule=if_let_to_matches
# lang=rust
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
@@ -40,1 +40,1 @@ rule=dedupe_assert_eq
# lang=rust
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
@@ -20,1 +20,1 @@ rule=duplicate_arg
# lang=rust
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

### 17.4 Repeated metavariable occurrences

A repeated metavariable occurrence is an equality constraint on the current candidate match.

The engine must bind the metavariable on its first occurrence and then require every later occurrence of the same metavariable to be structurally equal to the first bound value, as defined in Section 15.7.

This pattern:

```diff
@@ -40,1 +40,1 @@ rule=same_assert_args
# lang=rust
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
@@ -10,1 +10,1 @@ rule=unbox_many
# lang=rust
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

To rewrite both layers, the patch author must emit two ordered RSmPL hunks or use a future explicit repeat mode.

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
@@ -10,1 +10,1 @@ rule=call_two_args
# lang=rust
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


---

## 18. Rewrite Semantics

### 18.1 Replacement

A hunk with matching `-` and `+` patterns replaces the matched AST fragment.

> **v0.1 implementation note:** function-item hunks support signature evolution, not just renames. Each explicitly written slot evolves independently: an explicit `-`/`+` slot difference inserts, replaces, or deletes that slot's source bytes while all other slots (including the body and, unless `...` is replaced by concrete text, the parameter list) are preserved. Slot comparison is whitespace-insensitive against target text sliced from the original source; a concrete `-` slot must equal the target or the candidate is rejected (fail over guessing). `...` (parameters) and the same metavariable on both sides (visibility, qualifiers, generics, where) mean "preserve". A concrete body on either side is a compile error (bodies are always preserved). Minus-concrete/plus-omitted deletes the slot for parameters-as-`...`-removed, generics, and `where` clauses; for the return type, minus-concrete/plus-omitted deletes `-> T`, while plus-omitted with minus-`...` preserves the return type (rename fast-path). There is no syntax to delete a return type while matching any return with `...`; `# returns=none` constrains only the `-` side. Impl-block methods (`impl` context hunks) share the same slot semantics, including `self`/receiver parameters.
>
> Bare `<kind> <Name>` pairs for `struct`, `enum`, `trait`, `union`, `type`, `const`, `static`, `mod`, and `impl` (Section 14.8) are item renames, not general shapes: one ident-span edit per match, everything else preserved. `impl` renames the self-ty ident only.
>
> Non-call shapes are handled by the generalized matcher. Statements are limited to `let` bindings (`syn::Stmt::Local`, no `let-else`): the pattern may bind the whole pattern with one `# pat P` placeholder, use a concrete pattern, or spell an ascribed shape `P: T` with placeholder parts. A bare `P` never matches an annotated `x: T` (strict, like function slots); embedded metavariables elsewhere in the pattern are rejected at compile time. The initializer is matched with the general expression matcher.
>
> General expressions cover paths, literals, binary/unary/cast/reference/paren, calls, method calls, assignment, field access, indexing, arrays, tuples, struct literals, ranges, `await`, and `?`. General types cover paths with generic arguments, references, slices, arrays, tuples, parentheses, and `_` (which matches any type). Anything else (blocks, `if`/`match`/loops/closures, `let-else` targets, turbofish placeholders like `P::<...>`, `...` wildcards in these shapes) rejects the candidate rather than guessing. Lifetimes are wildcards in v1 and are not compared. An empty `+` side deletes a `let` statement (whole lines) or a `...;` expression statement; type patterns cannot be deleted. Leading `#[...]` attributes are only supported on `let` patterns and follow `# attrs=...` (a `+` side that writes attributes the `-` side did not is rejected as ambiguous). The fail-fast reference is Section 18.6.

Example:

```diff
@@ -40,8 +40,8 @@ rule=rename_call
# lang=rust
# expr E
- old_api(E)
+ new_api(E)
```

### 18.2 Deletion

A hunk with only `-` semantic pattern lines removes the matched AST fragment.

Example:

```diff
@@ -20,12 +20,0 @@ rule=delete_helper
# lang=rust
# item I
- I
```

### 18.3 Insertion

A hunk with only `+` semantic pattern lines inserts the new AST fragment at the new-side range.

Example:

```diff
@@ -5,0 +5,0 @@ rule=insert_use
# lang=rust
+ use crate::new_module;
```

### 18.4 Context-preserving rewrite

Context lines constrain the surrounding AST and are preserved.

Example:

```diff
@@ -20,12 +20,12 @@ rule=rename_method_inside_impl
# lang=rust
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
@@ -40,8 +40,8 @@ rule=method_to_function
# lang=rust
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
match found` and a suggestion carrying the exact `rsmpl-git --inspect
<file>` discovery command.
There is no partial matching and no guessing; check this table before
authoring instead of iterating against `--check`.

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
| C13 | `# symbol` declared without `# fallback=syntax` (no semantic environment in v0.1) | `` symbol `<Name>` could not be resolved (no semantic environment in v1) `` (code `RSMPL_SYMBOL_CONFLICT`, suggestion `` add `# fallback=syntax` to allow syntactic matching ``). Spelling note: the call-matcher symbol gate (Section 19 path) emits the same text with `v0.1` in place of `v1`; the table quotes the general-matcher spelling byte-for-byte. Unifying the two spellings is T24 follow-up. |

C6 note: `...;` expression-statement deletions never reach the general
matcher — call-like `foo(E);` shapes route to the call matcher (Section 13.4)
— so in practice C6 fires for every empty-`+` expression hunk. Delete a
`let` statement instead (whole lines), or write an explicit replacement.

Routing trap: `let (E, x) = ...` never reaches gate C8 — the tuple parens
make the call router claim it as a `let(...)` call pattern. Non-call shapes
such as `let [E, x] = ...` reach the `let` gate.

#### Match-time silent rejections (surface as `no match found`)

| # | Trigger | Mechanism | Write instead |
|---|---|---|---|
| R1 | block, `if`, `match`, loops, or closures as the general expression | bounded variant set: paths, literals, binary/unary/cast/reference/paren, calls, method calls, assignment, field access, indexing, arrays, tuples, struct literals, ranges, `await`, `?`; anything else rejects the candidate | an ordinary Git hunk, or a call/expr shape around the inner expression |
| R2 | `let-else` in the *target* file | v1 never matches `let-else` targets (`diverge.is_some()` rejects the candidate) | an ordinary Git hunk |
| R3 | turbofish placeholder (`P::<...>` — a placeholder path segment carrying generic arguments) | placeholder segments with arguments never match (concrete turbofish such as `::<u8>` compares by rendered text and works) | a concrete turbofish, or a whole-expression `# expr E` placeholder |
| R4 | `P @ ...` subpattern bindings in `let` | `subpat.is_some()` rejects the candidate | a whole-pattern `# pat P` placeholder or a concrete pattern |
| R5 | unsupported type variants (`impl Trait`, `dyn Trait`, fn-pointer types, macro types, …) | supported: paths with generic arguments, references, slices, arrays, tuples, parentheses, `_` (matches any type), `!`; anything else rejects the candidate | a concrete type or a whole-type `# type T` placeholder |

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
@@ -80,20 +80,20 @@ rule=rename_symbol_calls
# lang=rust
# allow=many
# symbol Old=crate::old_api
# symbol New=crate::new_api
- Old(...)
+ New(...)
```

The engine must:

1. find calls whose callee resolves to `crate::old_api`;
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

If `fallback=syntax` is set, the engine may use syntactic matching when semantic resolution is unavailable. This is less safe and should be discouraged for important refactors.

> **v0.1 implementation note:** syntactic fallback compares only the last path segment of the symbol (e.g. `crate::old_api` matches any callee whose final segment is `old_api`). Symbol hunks without `# fallback=syntax` are rejected because v0.1 has no semantic environment.

---

## 20. Moving Code

Moving code is one of the main motivations for RSmPL-Git.

A normal Git patch must repeat the removed and inserted code. RSmPL can capture an AST item and reinsert it elsewhere.

### 20.1 Cut item

```diff
diff --git a/src/old.rs b/src/old.rs
index 1111111..2222222 100644
--- a/src/old.rs
+++ b/src/old.rs
@@ -20,12 +20,0 @@ rule=cut_helper
# lang=rust
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
diff --git a/src/new.rs b/src/new.rs
index 3333333..4444444 100644
--- a/src/new.rs
+++ b/src/new.rs
@@ -5,0 +5,0 @@ rule=paste_helper
# lang=rust
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
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,0 +10,0 @@ rule=paste_helper_near_top
# lang=rust
# use=Helper
+ Helper

@@ -100,12 +100,0 @@ rule=cut_helper_from_bottom
# lang=rust
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
diff --git a/src/old.rs b/src/old.rs
index 1111111..2222222 100644
--- a/src/old.rs
+++ b/src/old.rs
@@ -20,12 +20,0 @@ rule=cut_helper
# lang=rust
# item I
# bind=Helper
- I

diff --git a/src/new.rs b/src/new.rs
index 3333333..4444444 100644
--- a/src/new.rs
+++ b/src/new.rs
@@ -5,0 +5,0 @@ rule=paste_helper
# lang=rust
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
@@ -30,6 +30,6 @@ rule=rename_macro
# lang=rust
# macro M
# token_tree TT
- M!(TT)
+ new_macro!(TT)
```

### 21.3 Token-tree rewrite

```diff
@@ -30,6 +30,6 @@ rule=rewrite_inside_macro
# lang=rust
# macro=token_tree
- old_token
+ new_token
```

This is less structured than AST mode and should be used cautiously.

### 21.4 Ellipsis in token-tree mode

In `# macro=token_tree` mode, `...` is a token-tree wildcard. It matches any sequence of zero or more tokens inside the current token-tree delimiter, subject to balanced delimiter preservation.

Example:

```diff
@@ -30,1 +30,1 @@ rule=wrap_macro_body
# lang=rust
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
@@ -30,1 +30,1 @@ rule=rename_macro_preserve_body
# lang=rust
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

> **v0.1 implementation note:** only `# format=preserve` is implemented. Non-`preserve` values are rejected with a diagnostic instead of being silently ignored.

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

If requested:

```text
# format=rustfmt
```

the engine may run `rustfmt` on the affected item, range, or file.

The implementation must document the formatting scope.

Recommended scopes:

```text
# format_scope=node
# format_scope=range
# format_scope=file
```

### 22.3 Canonical mode

```text
# format=canonical
```

lets the engine emit syntax using its own pretty-printer. This is useful for prototypes but less ideal for human-reviewed patches.

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
diff --git a/src/b.rs b/src/b.rs
...
@@ -20,0 +20,0 @@ rule=paste
# lang=rust
# use=Thing
+ Thing

diff --git a/src/a.rs b/src/a.rs
...
@@ -10,5 +10,0 @@ rule=cut
# lang=rust
# item I
# bind=Thing
- I
```

The engine must resolve `Thing` by evaluating the `# bind=Thing` hunk before materializing the `# use=Thing` insertion, even though the `# use` appears first in the patch file.

### 23.3 Upward moves in the same file

Forward capture references are especially important for moving code upward in a file.

A Git-like patch ordered by line number places the insertion near the top before the deletion near the bottom:

```diff
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,0 +10,0 @@ rule=paste_helper
# lang=rust
# use=Helper
+ Helper

@@ -100,12 +100,0 @@ rule=cut_helper
# lang=rust
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

This remains true even if hunk 1 requested:

```text
# format=rustfmt
```

and the formatted output has a different number of lines than the input.

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
2. RSmPL directive block lacks `# lang=rust`;
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
suggestion: run `rsmpl-git --inspect src/lib.rs` to list valid ranges
```

The `suggestion` carries the exact discovery command (repo-relative
path) so an agent repairs without guessing flags.

### 25.2 Parse conflict

Rust parsing failed.

```text
RSMPL_PARSE_CONFLICT
rule: rename_foo
file: src/lib.rs
reason: target file does not parse as Rust
```

### 25.3 Pattern conflict

The RSmPL pattern failed to parse.

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

A zero-match failure carries the exact discovery command instead:

```text
RSMPL_MATCH_CONFLICT
rule: rename_calls
file: src/lib.rs
old_range: 40..47
reason: no match found
suggestion: run `rsmpl-git --inspect src/lib.rs` (add `--calls` for call-rewrite ranges) to list valid ranges; check the pattern or add `# optional`
```

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
Inspect flag misuse (for example `--calls` without `--inspect`, or
`--inspect` combined with `--patch`/`--file`, `--check`/`--output`, or
`--diff-max-lines`/`--context`) is also reported under this code so JSON-mode
agents always receive a machine-readable envelope.

```text
RSMPL_IO_CONFLICT
file: change.rsmpl
reason: cannot read patch file change.rsmpl: No such file or directory (os error 2)
```

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
@@ -10,7 +10,7 @@ rule=rename_foo
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

The lowered hunks carry a unified-diff context radius: the number of
unchanged lines shown around each change. The default radius is 3, matching
conventional unified diffs. A smaller radius (down to `0`, which shows only
changed lines) spends fewer review tokens; per-file added/removed line counts
are computed from the full old/new contents and never change with the radius.
A zero-context lowering is still a valid unified diff, but plain
`git apply` rejects zero-context hunks by default as a safety measure — it
needs `git apply --unidiff-zero` (which the reference CLI passes
automatically for every check/apply). Lowering stays deterministic given the
same inputs plus the same radius (see D.2.2 for the `--context N` CLI lever).

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

Good:

```diff
@@ -40,8 +40,8 @@ rule=rename_calls
# lang=rust
# allow=many
# expr E
- old_api(E)
+ new_api(E)
```

### 28.3 Use ranges aggressively

Keep RSmPL ranges small.

Better:

```diff
@@ -80,15 +80,15 @@ rule=rename_one_block
```

Worse:

```diff
@@ -1,500 +1,500 @@ rule=rename_whole_file
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

For cross-scope rename safety:

```diff
# symbol Old=crate::old_api
# symbol New=crate::new_api
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

### 29.1 Rename a plain function

```diff
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,7 +10,7 @@ rule=rename_plain_foo
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
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -30,12 +30,12 @@ rule=rename_complex_foo
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
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -40,8 +40,8 @@ rule=rename_calls
# lang=rust
# allow=many
# expr E
- foo(E)
+ bar(E)
```

### 29.4 Method call to free function

```diff
diff --git a/src/service.rs b/src/service.rs
index 1111111..2222222 100644
--- a/src/service.rs
+++ b/src/service.rs
@@ -75,20 +75,20 @@ rule=method_to_free_function
# lang=rust
# allow=many
# expr Receiver
# expr Arg
- Receiver.old_method(Arg)
+ new_function(Receiver, Arg)
```

### 29.5 Semantic rename with symbol resolution

```diff
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -80,20 +80,20 @@ rule=rename_symbol_calls
# lang=rust
# allow=many
# symbol Old=crate::old_api
# symbol New=crate::new_api
- Old(...)
+ New(...)
```

### 29.6 Move a function between files

```diff
diff --git a/src/old.rs b/src/old.rs
index 1111111..2222222 100644
--- a/src/old.rs
+++ b/src/old.rs
@@ -20,12 +20,0 @@ rule=cut_helper
# lang=rust
# item I
# bind=Helper
- I

diff --git a/src/new.rs b/src/new.rs
index 3333333..4444444 100644
--- a/src/new.rs
+++ b/src/new.rs
@@ -5,0 +5,0 @@ rule=paste_helper
# lang=rust
# use=Helper
+ Helper
```

### 29.7 Mixed RSmPL and ordinary Git patch

```diff
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,7 +10,7 @@ rule=rename_foo
# lang=rust
- fn foo(...) {
+ fn bar(...) {
      ...
  }

@@ -40,8 +40,8 @@ rule=rename_calls
# lang=rust
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
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -3,0 +3,0 @@ rule=insert_use
# lang=rust
+ use crate::new_module::Thing;
```

### 29.9 Rewrite inside an impl block

```diff
diff --git a/src/service.rs b/src/service.rs
index 1111111..2222222 100644
--- a/src/service.rs
+++ b/src/service.rs
@@ -20,40 +20,40 @@ rule=rename_impl_method
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
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -100,30 +100,30 @@ rule=optional_old_api_cleanup
# lang=rust
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
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,1 +10,10 @@ rule=insert_large_block
# lang=rust
- old_function()
+ new_function_that_spans(
+     many,
+     many,
+     lines,
+ )

@@ -20,5 +20,5 @@ rule=rename_second_function
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
patch               ::= file_section*

file_section        ::= git_file_header file_metadata* hunk*

git_file_header     ::= "diff --git " path " " path newline

file_metadata       ::= metadata_line newline

hunk                ::= ordinary_hunk | rsmpl_hunk

ordinary_hunk       ::= hunk_header ordinary_hunk_body

rsmpl_hunk          ::= hunk_header directive_block rsmpl_hunk_body

hunk_header         ::= "@@ -" range " +" range " @@" hunk_header_tail? newline

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

metavariable_kind   ::= "identifier"
                      | "path"
                      | "symbol"
                      | "expr"
                      | "stmt"
                      | "item"
                      | "type"
                      | "pat"
                      | "lifetime"
                      | "generic_params"
                      | "where_clause"
                      | "visibility"
                      | "qualifier"
                      | "qualifier_list"
                      | "attributes"
                      | "block"
                      | "field"
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
- structured diagnostics in JSON;
- conflict markers for semantic patches;
- automatic import management;
- semantic move with path rewriting;
- stable canonical formatting mode;
- explicit collection-valued captures and list paste separators;
- explicit edition support:

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

The canonical RSmPL hunk shape is:

```diff
@@ -old_start,old_len +new_start,new_len @@ rule=name
# lang=rust
# optional-directive
# metavariable-kind Name
- old_pattern
+ new_pattern
```

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

1. parse Git-style file sections;
2. distinguish ordinary hunks from RSmPL hunks;
3. require `# lang=rust` for RSmPL hunks;
4. treat directive lines as metadata, not source lines;
5. derive file and range from Git patch structure;
6. restrict semantic matching to the declared old-side range;
7. support concrete Rust syntax matching;
8. support `...` in defined Rust syntactic positions;
9. support at least `expr`, `item`, `type`, `pat`, `identifier`, and `path` metavariables;
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

## Appendix D: Agent CLI Surface (stdin, JSON report, inspect)

The patch format above is engine-agnostic, but the reference `rsmpl-git`
binary exposes agent-oriented conveniences. They do not change patch
semantics; they reduce the tokens and round-trips an LLM needs to author
valid `@@ -old,len` ranges and verify results.

### D.1 Piped patches (stdin)

`--patch -` and `--file -` read the RSmPL-Git patch from stdin instead of
from an argument or a temp file:

```sh
cat change.rsmpl | rsmpl-git --patch - --repo ./my-repo
```

Both sources stay: `--patch` keeps small patches temp-file-free while
`--file` avoids shell-quoting for large patches; a unified
`<text|@file|->` spelling would add quoting ambiguity for zero token
savings.

### D.2 Machine-readable reports (`--format json`)

`--format text` (default) preserves the existing human output. `--format json`
prints one JSON object to stdout:

```json
{
  "rules": [{"rule": "rename_foo", "file": "src/lib.rs", "matches": 1}],
  "files": [{"path": "src/lib.rs", "added": 1, "removed": 1}],
  "diff": "diff --git ... (lowered concrete patch, no RSmPL directives)",
  "diff_truncated": false,
  "check": "ok"
}
```

- `rules` is the per-rule match report (text mode prints only one-line
  `applied`/`check` summaries; agents needing match counts use JSON).
- `files` lists per-file `{path, added, removed}` line stats computed from
  the full old/new contents, so counts stay exact even when `diff` is
  truncated or omitted (see D.2.1). Empty when no changes. `files[].path`
  is the single source of changed paths (a former `files_changed` key that
  duplicated these paths was removed as pure report overhead).
- `diff` is the lowered ordinary Git diff (empty string when no changes).
- `diff_truncated` is `true` iff the lowered diff is non-empty and the
  reported `diff` string hides any of it (`--diff-max-lines N` cut lines
  off, including `N=0` which omits); else `false` (including no-changes).
- `check` is `"ok"` when `--check` was passed, else `null`.
- `--output <path>` always writes the full diff file in both modes, even
  when the reported `diff` is truncated or omitted.
- Exit codes are unchanged: `0` applied/clean, `2` patch error with a
  structured `RSMPL_*`/`GIT_*` diagnostic on stderr.
- Failure envelope: with `--format json`, every patch/IO failure prints
  one JSON envelope to stdout and exits 2, so an agent can repair without
  parsing stderr prose:

```json
{"error": {"code": "RSMPL_MATCH_CONFLICT", "rule": "r", "file": "src/lib.rs", "range": "1..2", "reason": "no match found", "suggestion": "run `rsmpl-git --inspect src/lib.rs` (add `--calls` for call-rewrite ranges) to list valid ranges; check the pattern or add `# optional`"}}

```

- The envelope always carries all six keys (`code rule file range reason
  suggestion`); absent optionals are `null`. `code` is one of the SPEC 25
  codes, including `RSMPL_IO_CONFLICT` (25.8) for unreadable patch, repo,
  or output files, stdin failures, and inspect flag misuse (for example
  `--calls` without `--inspect`, or `--inspect` with `--patch`/`--file`,
  `--check`/`--output`, or `--diff-max-lines`/`--context`).
- Remaining flag-usage errors (no input at all, `--patch` plus `--file`
  together, clap parse errors) stay plain text on stderr in both modes;
  enveloping those is future work.

#### D.2.1 Output budget (`--diff-max-lines N`)

One large move can produce a lowered diff that exceeds an agent's context
all by itself. One apply-path flag bounds only the *reported* diff; it
never changes what is checked, applied, or saved:

```sh
# Stats + check only, no diff tokens at all:
rsmpl-git --file change.rsmpl --repo ./my-repo --format json --diff-max-lines 0 --check

# First 40 lines of diff for a quick skim, stats stay exact:
rsmpl-git --file change.rsmpl --repo ./my-repo --format json --diff-max-lines 40 --check

# Text equivalent (stdout is the reported diff, markers on stderr):
rsmpl-git --file change.rsmpl --repo ./my-repo --check --diff-max-lines 40
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
  `(no changes)` to stdout in both modes.
- `--check`, apply, and `--output` always use the FULL lowered diff, so
  `--format json --diff-max-lines 0 --check` verifies clean application
  with zero diff tokens, and `--diff-max-lines 1 --output full.patch`
  saves the appliable patch while showing one line. With no changes the
  flag is a no-op (`"diff": ""`, `"diff_truncated": false`, `"files": []`).
- The budget flag is rejected with `--inspect`
  (`--inspect does not take --diff-max-lines/--context`, exit 2, same
  `RSMPL_IO_CONFLICT` envelope in JSON / text block in text mode as other
  inspect flag misuse).
- To shrink the lowering itself rather than the report, use `--context N`
  (D.2.2): the budget flag above applies *after* lowering and composes with it.

#### D.2.2 Lowering context (`--context N`)

D.2.1 bounds the *reported* diff; `--context N` shapes the *lowered* diff
itself by setting the unified-diff context radius (default `3` when the flag
is absent, `0` allowed):

```sh
# Minimal tokens, still a real appliable patch:
rsmpl-git --file change.rsmpl --repo ./my-repo --check --context 0

# One context line for a quick skim; check still verifies the N-context diff:
rsmpl-git --file change.rsmpl --repo ./my-repo --format json --context 1 --check
```

Rules:

- `--context N` sets the number of unchanged lines shown around each lowered
  hunk. `--context 0` shows only changed (`-`/`+`) lines; hunk headers stay
  `@@ -a,b +c,d @@`. rsmpl-git's own `--check`/apply/`--output` path handles
  any N (it passes `git apply --unidiff-zero`, which plain `git apply`
  requires for zero-context hunks); piping a saved `--context 0` patch to a
  bare `git apply` needs that same flag.
- Unlike `--diff-max-lines`, `--context` changes what is checked, applied,
  and saved: `--check`, apply, and `--output` all see the N-context
  lowering. A truncated report (`diff_truncated: true`) is review-only,
  but an N-context diff is appliable — to recover full context, re-run
  without the flag.
- Per-file `{path, added, removed}` stats are computed
  from the full old/new contents, so they are identical for every N.
- Layering: the D.2.1 report budget applies after lowering. `--context 0
  --diff-max-lines 2` shows the first 2 lines of the 0-context diff;
  `--context 0 --diff-max-lines 0` omits with `diff_truncated: true` iff
  changes exist. With no changes `--context` is a no-op like the budget flag.
- `--context` is rejected with `--inspect` (same message and exit 2 as the
  D.2.1 flag, same `RSMPL_IO_CONFLICT` envelope); non-numeric values stay
  plain-text clap parse errors. The flag stays an `Option` (default applied
  as 3 when absent) rather than a clap `default_value_t` so the CLI can tell
  absent from explicitly passed (even an explicit `--context 3` is rejected
  with `--inspect`); hence the default is documented manually in the help
  text instead of clap's automatic `[default: 3]` marker.

### D.3 Range discovery (`--inspect <path>`)

`rsmpl-git --repo <dir> --inspect <rel-path> [--format json]` parses the
target file with the same Rust frontend the engine uses and lists its items
with 1-based inclusive line spans:

```sh
rsmpl-git --repo ./my-repo --inspect src/lib.rs
# fn foo 1-3
# struct Config 5-9
# method Config::load 11-15
```

JSON mode emits `{"path": ..., "items": [{"kind","name","start","end"}]}`.
Kinds are the closed set `fn struct enum trait impl type const static mod
use macro method union item extern_crate foreign_mod trait_alias` (any other
`syn::Item` variant falls back to `item`); `impl` methods are listed as
`method SelfTy::method` and inline `mod` contents recurse with `mod::item`
names. Spans cover the same attached attribute/doc envelope the matcher
sees, so an inspect span is directly usable as an `@@ -start,len` old-side
range.

#### D.3.1 Call-site preview (`--inspect <path> --calls`)

Item spans locate definitions, but call rewrites target call sites inside
function bodies. `--calls` additionally lists candidate call sites with
1-based inclusive line spans, so an agent can pick a tight `@@ -old,len`
range without reading the whole file:

```sh
rsmpl-git --repo ./my-repo --inspect src/lib.rs --calls
# fn foo 1-3
# call foo 5-5
# method try_send 5-5
# macro assert_eq 9-9
```

Call kinds:

| Kind | Source | `name` rendering |
|---|---|---|
| `call` | `Expr::Call` with a path callee | full path (`a::b::foo`) |
| `method` | `Expr::MethodCall` | method ident (`try_send`) |
| `macro` | `Expr::Macro` or `Stmt::Macro` invocation | full macro path (`assert_eq`) |

Rules:

- Text mode prints items first (unchanged), then call sites in ascending
  start-line order.
- JSON mode emits `{"path", "items", "calls"}` when `--calls` is set; without
  the flag the `calls` key is omitted (backward compatible).
- Only path-callee calls are listed (the shapes the engine's call matcher
  rewrites); non-path callees such as `(f())()` are skipped. Top-level
  `Item::Macro` definitions stay in `items` and are not duplicated in `calls`.
- Type usages are intentionally not listed (every annotation would emit;
  too noisy). Use a targeted file read for type-rewrite ranges.

#### D.3.2 Directory scan (`--inspect <dir>`)

Passing a directory (e.g. `--inspect src/`, `--inspect src`,
`--inspect .`) scans every `.rs` file beneath it in one call, so agents
skip `git ls-files` plus N per-file inspects:

```sh
rsmpl-git --repo ./my-repo --inspect src/ --calls
# src/lib.rs: fn foo 1-1
# src/lib.rs: call foo 3-3
# src/a.rs: struct S 1-2
```

Rules:

- Only `.rs` files are scanned. Dot-directories (`.git/`, …) and
  `target/` are skipped, as are non-`.rs` files. Symlinked directories
  are not followed.
- Files are processed in lexicographic repo-relative order; entries
  within a file keep the single-file order (items sorted, then calls
  sorted when `--calls` is set).
- Text mode prints grep-style `rel: kind name start-end` lines. Files
  with no entries emit no lines (token-efficient); use JSON mode to
  confirm coverage.
- JSON mode emits `{"path", "files": [{"path", "items"[, "calls"]}]}`
  with one entry per successfully parsed `.rs` file (empty `items: []`
  included). The per-file `calls` key follows the same omission rule as
  single-file mode. The single-file shape (`{"path", "items"…}`) is
  unchanged.
- Unreadable or unparseable files are skipped with a stderr
  `warning: skipping <rel>: …` so one broken file never blocks the
  scan; single-inspect that file for the full `RSMPL_PARSE_CONFLICT`.
- On huge repos scope to a subtree (`--inspect src/net/`) rather than
  the root, or bound the output with `--max-items N` (D.3.3).

#### D.3.3 Output budget (`--inspect <path> --max-items N`)

Directory scans and call previews are unbounded by default, which can
exceed an agent's context on large repos. `--max-items N` caps the total
number of listed entries (`items` + `calls` combined) in deterministic
sorted order (files lexicographic, items-then-calls within each file):

```sh
rsmpl-git --repo ./my-repo --inspect src --calls --max-items 20
rsmpl-git --repo ./my-repo --inspect src/lib.rs --max-items 5 --format json
```

Rules:

- `N` is a global cap, not per-file. The first `N` entries in the
  single/dir order are emitted; the rest are omitted. `--max-items 0`
  emits no entries (coverage/headers only). Absent flag means unbounded
  (backward compatible).
- Requires `--inspect`; otherwise the CLI exits 2 with
  `--max-items requires --inspect <path>` (same `RSMPL_IO_CONFLICT`
  envelope as `--calls` without `--inspect`).
- Text mode prints at most `N` entry lines to stdout (pure
  `kind name s-e` / `rel: kind name s-e` lines). When entries were
  omitted, one stderr marker is printed:
  `truncated: {omitted} more entries omitted (use --max-items N or scope to a subtree)`.
- JSON mode adds a top-level `"truncated": bool` to both shapes:
  single `{"path", "items"[, "calls"], "truncated"}` and directory
  `{"path", "files": [{"path", "items"[, "calls"]}], "truncated"}`.
  Truncated runs contain fewer entries (partial file arrays, later files
  omitted); empty files (`items: []`, plus `calls: []` when `--calls`)
  are still included for coverage because they cost no budget.
  Uncapped small outputs report `"truncated": false` so agents can always
  check the key. The per-file `calls` omission rule (D.3.1/D.3.2) is
  unchanged.

Recommended agent loop:

```text
inspect dir -> author hunks in original-source coordinates
-> apply with --format json --context 0 --diff-max-lines 0 --check -> review files[] stats
-> widen with --context 1 (or --diff-max-lines N / --output full.patch) only if needed
-> apply for real
```

New authors: start with `AGENTS.md` (one-page authoring reference); this
appendix and Sections 9/18.6 are the normative detail behind it.

---
