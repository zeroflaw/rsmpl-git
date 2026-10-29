# example-benches — RSmPL vs native Git patch size

`applyrs/NN-name/` holds a `repo/` (pre-state), a `patch.rsmpl` (the agent-authored
RSmPL input), and a `native.patch` (the lowered 0-context Git diff the tool
emits — i.e. the smallest native patch producing the same result).
`diffrs/NN-name/` holds a `repo/` (pre-state) and an `after/` (post-state);
`native.patch` is the `git diff -U0` baseline and `sem.patch` the `diffrs`
compression of the same change.

`native.patch` is the fair baseline: same end state, full Git spelling.
An agent hand-writing the native patch with conventional 3-line context would
emit *more* lines, so the savings below are lower bounds.

Rebuild + re-measure: `./example-benches/run.sh` (from the repo root).
It re-applies every applyrs `patch.rsmpl`, rewrites every `native.patch`,
prints the applyrs table embedded in the top-level README, then re-measures
every diffrs bench as native vs semantic bytes (second table, embedded
below).

| bench | what it shows | rsmpl | native | saved |
|---|---|---|---|---|
| 01-rename-calls-many | 1 hunk rewrites 12 call sites (`allow=many`) | 83 B / 5 ln | 409 B / 21 ln | 80% |
| 02-move-fn-cross-file | cut+paste a 44-line fn, body never restated | 142 B / 9 ln | 1576 B / 97 ln | 91% |
| 03-scope-file-sweep | `scope=file` rewrites 9 sites, no range discovery | 83 B / 5 ln | 445 B / 20 ln | 81% |
| 04-method-to-function | receiver/arg metavars over 4 sites | 134 B / 5 ln | 347 B / 15 ln | 61% |
| 05-use-prefix | 1 hunk moves 5 imports after a `mod` rename | 98 B / 5 ln | 347 B / 14 ln | 72% |
| 06-repeat-fold | `repeat` folds nested `Box::new` in one hunk | 85 B / 6 ln | 209 B / 9 ln | 59% |
| 07-let-and-type | `let` + `Vec<T>`→`Box<[T]>` over 7 sites, 3 hunks | 208 B / 15 ln | 593 B / 21 ln | 65% |
| 08-rename-fn-big-body | rename over a 54-line body (`...` keeps it) | 110 B / 6 ln | 149 B / 6 ln | 26% |
| 09-sig-evolution | evolve 2 signatures, bodies preserved | 202 B / 12 ln | 251 B / 9 ln | 20% |
| 10-item-rename | 3 bare-pair item renames in one file | 189 B / 12 ln | 210 B / 12 ln | 10% |
| 11-repeat-general-expr | `repeat` folds nested `E + 0` in one hunk (general-expr shape) | 82 B / 6 ln | 213 B / 11 ln | 62% |
| 12-amp-receiver | `&E.join("")` to `E` over 4 sites, 1 hunk (general-expr `&` bypass) | 75 B / 5 ln | 255 B / 12 ln | 71% |
| 13-call-sweep-scaling | 1 hunk rewrites 20 call sites, `scope=file` keeps input constant as native grows | 77 B / 5 ln | 649 B / 33 ln | 88% |
| 14-slot-sweep | `V`/`Q`/`G`/`W` rename `foo`→`bar` over 4 heterogeneous signatures, slots preserved | 122 B / 7 ln | 405 B / 15 ln | 70% |
| 15-pat-sweep | `P`/`T` preserve 4 ascribed patterns+types while `E`→`E.wrapping_add(1)` | 105 B / 5 ln | 331 B / 12 ln | 68% |
| 16-symbol-sweep | `Old`→`New` rewrites 5 qualified `crate::old_api` sites, bare/other distractors untouched | 152 B / 8 ln | 425 B / 14 ln | 64% |
| 17-macro-sweep | `old_mac!`→`new_mac!` over 5 sites, `other_mac!` distractor untouched, 1 hunk | 110 B / 6 ln | 285 B / 14 ln | 61% |
| 18-attrs-sweep | `V`/`Q`/`G`/`W` rename `foo`→`baz` over 4 attributed sigs, attrs preserved, `bar` untouched, 1 hunk | 122 B / 7 ln | 407 B / 15 ln | 70% |
| 19-impl-sweep | 1 hunk renames `old_name`→`new_name` over 4 heterogeneous `impl` blocks, `;` decl + free fn untouched | 156 B / 10 ln | 391 B / 15 ln | 60% |
| 20-trait-sweep | 1 hunk renames `old_name`→`new_name` over 3 heterogeneous `trait` blocks (`;` decls + default body), `impl` + free fn untouched | 124 B / 7 ln | 303 B / 12 ln | 59% |

Reading the table: fan-out edits (many sites, moves) save 50–90%.
Diet spelling (`# sweep`, inferred metavars, terse renames) keeps even
single-ident edits (09, 10) byte-positive — plus semantic safety
(slot-exact matching, attrs/doc-comments preserved, machine-checked
counts in the JSON report).

| bench | native | semantic | saved |
|---|---|---|---|
| 01-call-sweep (12 call sites, 1 hunk) | 416 B / 28 ln | 75 B / 5 ln | 82% |
| 02-ordinary-edit (1 line, stays ordinary) | 89 B / 6 ln | 72 B / 6 ln | 19% |
| 03-move (same-file reorder, cut+paste) | 1857 B / 89 ln | 106 B / 9 ln | 94% |
| 04-peel (6 call sites, 2 peel hunks) | 1006 B / 22 ln | 425 B / 18 ln | 58% |
| 05-stranded (3 sites, stranded join + insertion) | 349 B / 15 ln | 196 B / 11 ln | 44% |
| 06-literal-cross-hunk (2 sites, section driver) | 679 B / 19 ln | 464 B / 17 ln | 32% |
| 07-ref-strip (2 sites, ref-strip sweep) | 212 B / 8 ln | 90 B / 5 ln | 58% |
| 08-slot-sweep (2 fns, slot sweep) | 540 B / 9 ln | 146 B / 7 ln | 73% |
| 09-arg-add (3 sites, arg-add sweep + hetero ordinary) | 489 B / 18 ln | 295 B / 13 ln | 40% |
| 10-same-literal (3 sites, same-literal sweep) | 386 B / 10 ln | 148 B / 5 ln | 62% |
| 11-let-sweep (2 let sites, verbatim sweep) | 319 B / 9 ln | 185 B / 7 ln | 42% |
| 12-verbatim (2 sites, verbatim sweep) | 199 B / 9 ln | 123 B / 7 ln | 38% |
| 13-verbatim-return (2 sites, Return-arm verbatim sweep) | 266 B / 9 ln | 146 B / 7 ln | 45% |
| 14-verbatim-struct-field (2 sites, field-label verbatim sweep) | 281 B / 9 ln | 127 B / 7 ln | 55% |
| 15-verbatim-unary (2 sites, Unary-arm verbatim sweep) | 219 B / 9 ln | 123 B / 7 ln | 44% |
| 16-call-literal (2 sites, literal sweep + hetero ordinary) | 548 B / 16 ln | 361 B / 13 ln | 34% |
| 17-macro-veto (6 sites, macro gates stay ordinary) | 1208 B / 24 ln | 1040 B / 24 ln | 14% |
| 18-macro-invocation-veto (2 sites + pure addition, stays ordinary) | 378 B / 14 ln | 308 B / 14 ln | 19% |
| 19-ref-strip-scale (5 sites, chained-receiver ref-strip sweep) | 436 B / 14 ln | 86 B / 5 ln | 80% |
| 20-method-to-function (3 sites, method-to-function sweep) | 237 B / 10 ln | 109 B / 5 ln | 54% |

`diffrs/` benches compare `git diff -U0` (`native.patch`, `index` lines
stripped) against the `diffrs` compression (`sem.patch`) for the same end
state.
