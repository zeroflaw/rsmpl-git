# Demo 04: live agent, with vs without gitrs

Minimum harness for a headless `muse exec` agent turn. Each invocation runs
the agent once, either **with** the gitrs skill (`--with-gitrs`, default)
or **without** it (`--without-gitrs`), streams the agent's messages live,
and reports runtime, token/context usage, and a compaction count.

The two arms differ **only** in the skill staged into the temp workspace,
a directive block appended to the prompt, and the binary dir on `PATH`
(with-arm only — the without-arm leaves it off and strips any inherited
entry pointing at it, so `gitrs` does not resolve). Model, workspace, and
flags are otherwise identical.

The task: a fresh clone of hashbrown v0.17.1 (pinned commit
`c62a63a6`) lands in `workspace/hashbrown/`, and the agent must rename the
crate-private scope-guard helper — type `ScopeGuard` → `DropGuard`,
function `guard` → `drop_guard`, associated function `into_inner` →
`into_value`, field `dropfn` → `drop_fn` — updating every code use across
the crate, plus moving the four `util.rs` helpers into `scopeguard.rs`
behind a re-export shim (5 files, 58+/55-) — then prove `cargo check`
passes (default, `--features rayon`, `--features raw-entry`, `--tests`,
and `--features nightly`) plus `cargo test --lib`. The prompt brief is
derived from the reference patch in `assets/expected/patch.rsmpl` (68
lines), whose lowered native diff is kept alongside as
`assets/expected/native.diff`; each run captures the agent's own diff as
`runs/<stamp>/agent.diff` for comparison.

## Prerequisites

- `muse` CLI on `PATH`, model `muse-spark-1.3` available to your key.
- An LLM key (one of):
  `export META_API_KEY=...` (or `MUSE_KEY=...`),
  `--api-key VALUE`, `--api-key-file PATH`, `--api-key-env NAME`.
- `cargo` + `git` (release binaries are rebuilt on every run via
  `cargo build --release -p gitrs -p applyrs -p diffrs`; cargo no-ops fast
  when the tree is fresh, so the agent never tests a stale engine).

## Run

From the repo root:

```sh
python3 demo/04-live-agent-refactor/run.py --with-gitrs
python3 demo/04-live-agent-refactor/run.py --without-gitrs
```

Useful flags: `--disable-sandbox`, `--contributor-mode on|off`,
`--approval-mode never` (unattended runs may stall on approvals otherwise),
`--max-model-steps N`, `--model ID` (default `muse-spark-1.3`), `--verbose`
(print every event and echo stderr live).

## What a run produces

- Live on stdout: the agent's message text as it streams, plus one
  `… tool <name> → <outcome>` line per tool call. Raw events and the
  child's stderr (including the provider trace) go to the run dir only;
  `--verbose` also prints them live.
- `runs/<stamp>-<mode>/`: `prompt.txt`, `argv.txt`, `events.jsonl` (every
  stdout line, parsed or raw, with arrival timestamps), `stderr.log`
  (includes the provider transport trace by default), `summary.json`,
  plus `agent.diff` / `agent-status.txt` (the refactor result, captured
  before `workspace/` is removed — diff it against
  `assets/expected/native.diff`).
- End-of-run summary: runtime, exit code, event counts, observed
  token/context fields, compaction count.

Token/context figures come from the run's `run.usage` event (plus any
other token-ish fields, best-effort); the full record stays in
`events.jsonl` for re-mining. The task is small enough that
auto-compaction is unlikely; `compactions: 0` is the expected reading.

## Layout

- `run.py` — the harness (single code path; `with_gitrs` gates exactly the
  skill staging and the directive append, nothing else).
- `assets/prompts/base.txt` — the task brief (shared by both arms).
- `assets/prompts/gitrs-directive.txt` — appended only in `--with-gitrs`.
- `assets/skills/gitrs/SKILL.md` — verbatim copy of
  `skills/gitrs/SKILL.md` minus the `## Dogfood rule` section (staged to
  `workspace/.agents/skills/gitrs/SKILL.md` in `--with-gitrs` only).
- `assets/expected/patch.rsmpl` — reference semantic patch (68 lines,
  `--check`-verified; replays byte-identically from a fresh clone).
- `assets/expected/native.diff` — the ordinary `git diff` the reference
  patch lowers to (5 files, 58+/55-; `cargo check` passes default +
  `--features rayon` + `--features raw-entry` + `--tests` +
  `--features nightly`, and `cargo test --lib` is 105/105).
- `workspace/` — temp agent workspace, reset every run; holds the pinned
  `hashbrown/` clone plus the staged skill (with-arm). Gitignored.
- `runs/` — per-run transcripts and summaries. Gitignored.
