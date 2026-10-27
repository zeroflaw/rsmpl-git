# example-benches — RSmPL vs native Git patch size

Each `NN-name/` holds a `repo/` (pre-state), a `patch.rsmpl` (the agent-authored
RSmPL input), and a `native.patch` (the lowered 0-context Git diff the tool
emits — i.e. the smallest native patch producing the same result).

`native.patch` is the fair baseline: same end state, full Git spelling.
An agent hand-writing the native patch with conventional 3-line context would
emit *more* lines, so the savings below are lower bounds.

Rebuild + re-measure: `./example-benches/run.sh` (from the repo root).
It re-applies every `patch.rsmpl`, rewrites every `native.patch`, and prints
the table embedded in the top-level README.

| bench | what it shows | rsmpl | native | saved |
|---|---|---|---|---|
| 01-rename-calls-many | 1 hunk rewrites 12 call sites (`allow=many`) | 92 B / 6 ln | 409 B / 21 ln | 78% |
| 02-move-fn-cross-file | cut+paste a 44-line fn, body never restated | 151 B / 10 ln | 1576 B / 97 ln | 90% |
| 03-scope-file-sweep | `scope=file` rewrites 9 sites, no range discovery | 110 B / 7 ln | 445 B / 20 ln | 75% |
| 04-method-to-function | receiver/arg metavars over 4 sites | 161 B / 7 ln | 347 B / 15 ln | 54% |
| 05-use-prefix | 1 hunk moves 5 imports after a `mod` rename | 98 B / 5 ln | 347 B / 14 ln | 72% |
| 06-repeat-fold | `repeat` folds nested `Box::new` in one hunk | 94 B / 7 ln | 209 B / 9 ln | 55% |
| 07-let-and-type | `let` + `Vec<T>`→`Box<[T]>` over 7 sites, 3 hunks | 226 B / 17 ln | 593 B / 21 ln | 62% |
| 08-rename-fn-big-body | rename over a 54-line body (`...` keeps it) | 122 B / 7 ln | 149 B / 6 ln | 18% |
| 09-sig-evolution | evolve 2 signatures, bodies preserved | 253 B / 14 ln | 251 B / 9 ln | -1% |
| 10-item-rename | 3 bare-pair item renames in one file | 225 B / 15 ln | 210 B / 12 ln | -7% |
| 11-repeat-general-expr | `repeat` folds nested `E + 0` in one hunk (general-expr shape) | 109 B / 8 ln | 213 B / 11 ln | 49% |
| 12-amp-receiver | `&E.join("")` to `E` over 4 sites, 1 hunk (general-expr `&` bypass) | 102 B / 7 ln | 255 B / 12 ln | 60% |
| 13-call-sweep-scaling | 1 hunk rewrites 20 call sites, `scope=file` keeps input constant as native grows | 104 B / 7 ln | 649 B / 33 ln | 84% |
| 14-slot-sweep | `V`/`Q`/`G`/`W` rename `foo`→`bar` over 4 heterogeneous signatures, slots preserved | 210 B / 12 ln | 405 B / 15 ln | 48% |
| 15-pat-sweep | `P`/`T` preserve 4 ascribed patterns+types while `E`→`E.wrapping_add(1)` | 149 B / 9 ln | 331 B / 12 ln | 55% |

Reading the table: fan-out edits (many sites, moves) save 50–90%.
Single-ident edits (09, 10) are size-parity — there the payoff is not bytes
but semantic safety (slot-exact matching, attrs/doc-comments preserved,
machine-checked counts in the JSON report) at ~zero token premium.
