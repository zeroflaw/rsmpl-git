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

Expect: live agent messages answering (1) gitrs skill visible, (2) `which
gitrs` output, (3) one-line purpose; then a `==== run summary ====` block
with runtime, event counts, and `compactions: 0`.

## 2. Without-gitrs arm

```sh
python3 demo/04-live-agent-refactor/run.py --without-gitrs
```

Expect: same shape, but answers (1) and (2) should say neither the
skill nor the binaries are available — the binary dir is left off `PATH`
in this arm (any inherited entry pointing at it is stripped). Answer (3)
should state gitrs is unavailable, not describe using it.

## 3. Inspect the artifacts

```sh
ls demo/04-live-agent-refactor/runs/
cat demo/04-live-agent-refactor/runs/*/summary.json
```

`events.jsonl` holds every stdout line with arrival timestamps (re-mine it
if `usage_observed` came back empty — the `--json` schema is
undocumented); `stderr.log` holds the transport trace. `workspace/` is
removed at the end of the run.

## 4. Iterate

- Tweak the prompt: edit `assets/prompts/base.txt` (the directive still
  appends in with-arm).
- Audit a run later: the printed `session:` id works with
  `muse export --session <uuid>`.
