# Demo 04 (manual): live agent with/without gitrs

Same harness as `run.py`, step by step. One headless `muse exec` turn per
invocation; the with/without arms differ only by the staged skill plus a
prompt directive.

## 0. Prerequisites

- `muse` on `PATH` (`muse --version`), key available as `META_API_KEY`
  (or `MUSE_KEY`, `--api-key`, `--api-key-file`).
- `cargo` on `PATH` (every run rebuilds `gitrs`/`applyrs`/`diffrs`;
  incremental, so fresh trees finish in seconds).
- From the repo root.

## 1. With-gitrs arm

```sh
export META_API_KEY="$MUSE_KEY"
python3 demo/04-live-agent-refactor/run.py --with-gitrs
```

Expect: the harness clones pinned hashbrown into `workspace/`, then the
agent performs the renames and the helper move via the gitrs skill,
runs the five `cargo check` variants plus the lib test suite, and finishes
with a `==== run summary ====` block.

## 2. Without-gitrs arm

```sh
python3 demo/04-live-agent-refactor/run.py --without-gitrs
```

Expect: same task, hand-edited without the skill or binaries (both
unavailable in this arm). Compare its runtime/token spend against the
with-arm run.

## 3. Inspect the artifacts

```sh
ls demo/04-live-agent-refactor/runs/
cat demo/04-live-agent-refactor/runs/*/summary.json
diff demo/04-live-agent-refactor/assets/expected/native.diff \
     demo/04-live-agent-refactor/runs/*/agent.diff
```

`events.jsonl` holds every stdout line with arrival timestamps;
`stderr.log` holds the transport trace; `agent.diff` holds the refactor
result (compare it against `assets/expected/native.diff` — same end
state, however the agent got there). `workspace/` is removed at the end
of the run.

## 4. Iterate

- Tweak the task: edit `assets/prompts/base.txt` (the directive still
  appends in with-arm). If you change the end state, re-derive
  `assets/expected/` to match.
- Audit a run later: the printed `session:` id works with
  `muse export --session <uuid>`.
