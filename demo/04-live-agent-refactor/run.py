#!/usr/bin/env python3
"""Demo 04: live-agent refactor harness (minimum viable version).

Runs ONE headless `muse exec` turn per invocation, either WITH the gitrs
skill (`--with-gitrs`, the default) or WITHOUT it (`--without-gitrs`).
Streams the agent's messages in real time, then reports runtime, token /
context usage, and a compaction count.

The two arms differ ONLY in:
  (a) the gitrs skill staged into the temp workspace,
  (b) a directive block appended to the prompt, and
  (c) the gitrs binary dir on PATH (with-arm only; the without-arm
      additionally strips any inherited entry pointing at it).
Model, workspace, and flags are otherwise identical.

Temp workspace: demo-dir/workspace/ (holds a fresh clone of pinned hashbrown plus the staged skill).
It is deleted at the start and at the end of every run.

Needs an LLM key at run time (child env META_API_KEY, never logged):
  --api-key <value> | --api-key-file <path> | --api-key-env <NAME>
  or export META_API_KEY (else MUSE_KEY).
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import shutil
import subprocess
import sys
import threading
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path

DEMO_DIR = Path(__file__).resolve().parent
ASSETS = DEMO_DIR / "assets"
WORKSPACE = DEMO_DIR / "workspace"
RUNS_DIR = DEMO_DIR / "runs"
ROOT = DEMO_DIR.parent.parent

DEFAULT_MODEL = "muse-spark-1.3"
HASHBROWN_URL = "https://github.com/rust-lang/hashbrown"
HASHBROWN_REV = "c62a63a61b7caf2de8f9ecb7b06a66b0ab6bdf3d"  # v0.17.1
HASHBROWN_DIR = "hashbrown"
BASE_PROMPT = ASSETS / "prompts" / "base.txt"
DIRECTIVE_PROMPT = ASSETS / "prompts" / "gitrs-directive.txt"
STAGED_SKILL_SRC = ASSETS / "skills" / "gitrs" / "SKILL.md"
SKILL_DST_REL = Path(".agents/skills/gitrs/SKILL.md")

GITRS_BINS = ("gitrs", "applyrs", "diffrs")
DISPLAY_TRUNCATE = 4000  # chars; full data always lands in events.jsonl


def parse_args(argv=None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description="Run one headless muse exec turn with/without the gitrs skill."
    )
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--with-gitrs", dest="with_gitrs", action="store_true",
                      default=True, help="stage the gitrs skill + directive (default)")
    mode.add_argument("--without-gitrs", dest="with_gitrs", action="store_false",
                      help="bare arm: no skill, no directive")
    p.add_argument("--model", default=DEFAULT_MODEL,
                   help=f"model id (default: {DEFAULT_MODEL})")
    p.add_argument("--disable-sandbox", action="store_true",
                   help="pass --disable-sandbox to muse exec (default: sandbox on)")
    p.add_argument("--contributor-mode", choices=("on", "off"), default=None,
                   help="pass --contributor-mode on|off (default: not passed)")
    p.add_argument("--approval-mode", choices=("unrestricted", "on-request", "never"),
                   default=None,
                   help="pass --approval-mode (unattended runs may need 'never'; "
                        "default: not passed, muse default applies)")
    p.add_argument("--max-model-steps", type=int, default=None, metavar="N",
                   help="cap the run's model steps")
    p.add_argument("--api-key", default=None, metavar="VALUE",
                   help="LLM key value (prefer env/file: argv leaks into shell history)")
    p.add_argument("--api-key-file", default=None, metavar="PATH",
                   help="read the LLM key from this file")
    p.add_argument("--api-key-env", default=None, metavar="NAME",
                   help="env var holding the key (default: META_API_KEY, then MUSE_KEY)")
    p.add_argument("--session-id", default=None, metavar="UUID",
                   help="muse session id (default: random; enables muse export --session)")
    p.add_argument("--gitrs-bin", default=None, metavar="DIR",
                   help="dir with gitrs/applyrs/diffrs (default: <root>/target/release)")
    p.add_argument("--no-build", action="store_true",
                   help="do not cargo-build missing gitrs binaries (fail instead)")
    p.add_argument("--no-transport-trace", dest="transport_trace",
                   action="store_false", default=True,
                   help="do not set MUSE_TRANSPORT_TRACE=1 (trace defaults ON; "
                        "captured to the run dir for usage calibration)")
    p.add_argument("--verbose", action="store_true",
                   help="print every JSONL event and echo child stderr live "
                        "(default: agent text + one line per tool call only)")
    return p.parse_args(argv)


def assemble_prompt(with_gitrs: bool) -> str:
    """Assembled prompt. The arms differ ONLY by the directive block."""
    base = BASE_PROMPT.read_text()
    if not base.endswith("\n"):
        base += "\n"
    if not with_gitrs:
        return base
    directive = DIRECTIVE_PROMPT.read_text()
    if not directive.endswith("\n"):
        directive += "\n"
    return base + "\n" + directive


def resolve_api_key(args: argparse.Namespace) -> tuple[str | None, str]:
    """Return (key, source-label). The label never carries the value."""
    if args.api_key:
        return args.api_key.strip(), "cli:--api-key"
    if args.api_key_file:
        return Path(args.api_key_file).read_text().strip(), f"file:{args.api_key_file}"
    names = [args.api_key_env] if args.api_key_env else ["META_API_KEY", "MUSE_KEY"]
    for name in names:
        value = os.environ.get(name)
        if value and value.strip():
            return value.strip(), f"env:{name}"
    return None, "missing"


def ensure_gitrs_bins(args: argparse.Namespace) -> Path:
    """Build the release binaries, then return the bin dir (unless --no-build)."""
    bindir = Path(args.gitrs_bin) if args.gitrs_bin else ROOT / "target" / "release"

    if not args.no_build:
        print("(building gitrs + applyrs + diffrs …)", flush=True)
        r = subprocess.run(
            ["cargo", "build", "--release", "-p", "gitrs", "-p", "applyrs",
             "-p", "diffrs"],
            cwd=str(ROOT),
            check=False,
        )
        if r.returncode != 0:
            sys.exit(f"error: cargo build failed (exit {r.returncode})")
    missing = [b for b in GITRS_BINS if not (bindir / b).is_file()]
    if missing:
        sys.exit(f"error: missing binaries in {bindir}: {', '.join(missing)} "
                 f"(omit --no-build to build, or pass --gitrs-bin DIR)")
    return bindir


def _strip_bindir_from_path(path_value: str, bindir: Path) -> str:
    """Drop PATH entries pointing at bindir (without-arm: gitrs must not resolve)."""
    try:
        target = bindir.resolve()
    except OSError:
        target = bindir.absolute()
    kept = []
    for entry in path_value.split(os.pathsep):
        if not entry:
            continue
        try:
            if Path(entry).resolve() == target:
                continue
        except OSError:
            pass
        kept.append(entry)
    return os.pathsep.join(kept)


def reset_workspace(with_gitrs: bool) -> None:
    """Delete workspace/ if present, recreate, stage the skill (with-arm only)."""
    if WORKSPACE.exists():
        shutil.rmtree(WORKSPACE)
    WORKSPACE.mkdir(parents=True)
    if with_gitrs:
        if not STAGED_SKILL_SRC.is_file():
            sys.exit(f"error: staged skill missing: {STAGED_SKILL_SRC}")
        dst = WORKSPACE / SKILL_DST_REL
        dst.parent.mkdir(parents=True)
        shutil.copyfile(STAGED_SKILL_SRC, dst)
    # NOTE: the pinned hashbrown clone lands here next (clone_hashbrown);
    # it is the agent's refactor target. Nothing else stages files into it.


def clone_hashbrown() -> Path:
    """Fresh-clone pinned hashbrown into the workspace; prefetch deps best-effort."""
    dest = WORKSPACE / HASHBROWN_DIR
    print(f"(cloning hashbrown@{HASHBROWN_REV[:12]} …)", flush=True)
    r = subprocess.run(["git", "clone", "--quiet", HASHBROWN_URL, str(dest)], check=False)
    if r.returncode != 0:
        sys.exit(f"error: git clone {HASHBROWN_URL} failed (exit {r.returncode})")
    r = subprocess.run(["git", "-C", str(dest), "checkout", "--quiet", HASHBROWN_REV],
                       check=False)
    if r.returncode != 0:
        sys.exit(f"error: git checkout {HASHBROWN_REV[:12]} failed (exit {r.returncode})")
    # Best-effort prefetch: the agent later runs `cargo check` inside the
    # sandbox, where registry access may be restricted. Warming the shared
    # cargo cache here (full network) maximizes the odds plain `cargo check`
    # succeeds there. Failure only warns; the agent may still manage.
    r = subprocess.run(["cargo", "fetch", "--quiet"], cwd=str(dest), check=False,
                       capture_output=True, text=True)
    if r.returncode != 0:
        print("(warning: cargo fetch failed; the agent's cargo check may need network)",
              flush=True)
    return dest


def build_argv(args: argparse.Namespace, prompt_path: Path, session_id: str) -> list[str]:
    argv = ["muse", "exec", "--json",
           "--model", args.model,
           "--workspace", str(WORKSPACE),
           "--trust-workspace",           # required: else project skills don't load
           "--no-foreign-personal-context",  # keep the arms comparable
           "--prompt-file", str(prompt_path),
           "--session-id", session_id]
    if args.disable_sandbox:
        argv.append("--disable-sandbox")
    if args.contributor_mode:
        argv += ["--contributor-mode", args.contributor_mode]
    if args.approval_mode:
        argv += ["--approval-mode", args.approval_mode]
    if args.max_model_steps is not None:
        argv += ["--max-model-steps", str(args.max_model_steps)]
    return argv


# --- --json parsing: `muse exec --json` emits session event-log records
# ({payload_type, payload, ...}), calibrated against a real run. Shapes used:
#   run.output.delta  -> payload.text chunks (the streaming agent message)
#   tool.result       -> payload.correlation_facts.{tool_name, outcome}
#   run.usage         -> payload.{calls, ..., models[]}
#   run.terminal.*    -> payload.text repeats the full answer (deltas cover it)
# Everything is still stored verbatim in events.jsonl; unknown shapes print
# only under --verbose.

def _usage_from_run_usage(payload: dict, acc: dict) -> None:
    """Tidy extraction from the known run.usage shape."""
    for key in ("calls", "completed_calls", "failed_calls", "cancelled_calls"):
        if isinstance(payload.get(key), int):
            acc[key] = payload[key]
    models = payload.get("models")
    if isinstance(models, list):
        acc["models"] = [
            {k: m.get(k) for k in
             ("model", "calls", "input_tokens", "output_tokens",
              "cached_tokens", "reasoning_tokens")}
            for m in models if isinstance(m, dict)
        ]


def _walk_usage(node, path: str, acc: dict, depth: int = 0) -> None:
    if depth > 6:
        return
    if isinstance(node, dict):
        for key, value in node.items():
            name = str(key)
            child = f"{path}.{name}" if path else name
            if (isinstance(value, (int, float)) and not isinstance(value, bool)
                    and any(s in name.lower() for s in ("token", "usage", "context"))):
                acc[child] = value
            else:
                _walk_usage(value, child, acc, depth + 1)
    elif isinstance(node, list):
        for i, value in enumerate(node):
            _walk_usage(value, f"{path}[{i}]", acc, depth + 1)


def _mentions_compaction(node, depth: int = 0) -> bool:
    if depth > 6:
        return False
    if isinstance(node, str):
        return "compact" in node.lower()
    if isinstance(node, dict):
        return any("compact" in str(k).lower() or _mentions_compaction(v, depth + 1)
                   for k, v in node.items())
    if isinstance(node, list):
        return any(_mentions_compaction(v, depth + 1) for v in node)
    return False


def main(argv=None) -> int:
    args = parse_args(argv)
    with_gitrs = args.with_gitrs
    mode = "with-gitrs" if with_gitrs else "without-gitrs"

    key, key_source = resolve_api_key(args)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    session_id = args.session_id or str(uuid.uuid4())
    run_dir = RUNS_DIR / f"{stamp}-{mode}"
    prompt_path = run_dir / "prompt.txt"

    if not key:
        sys.exit("error: no LLM key found. Provide one via --api-key VALUE, "
                 "--api-key-file PATH, --api-key-env NAME, or export META_API_KEY "
                 "(else MUSE_KEY).")

    bindir = ensure_gitrs_bins(args)
    run_dir.mkdir(parents=True)
    reset_workspace(with_gitrs)
    hb_dir = clone_hashbrown()
    prompt_path.write_text(assemble_prompt(with_gitrs))
    child_argv = build_argv(args, prompt_path, session_id)
    (run_dir / "argv.txt").write_text(shlex.join(child_argv) + "\n")

    env = dict(os.environ)
    env["META_API_KEY"] = key
    if with_gitrs:
        env["PATH"] = str(bindir) + os.pathsep + env.get("PATH", "")
    else:
        env["PATH"] = _strip_bindir_from_path(env.get("PATH", ""), bindir)
    if args.transport_trace:
        env["MUSE_TRANSPORT_TRACE"] = "1"

    print(f"mode: {mode} | model: {args.model} | session: {session_id}", flush=True)
    print(f"run dir: {run_dir}", flush=True)

    envelopes: list[dict] = []
    stderr_lines: list[str] = []
    usage: dict = {}
    compaction_indices: list[int] = []
    print_lock = threading.Lock()
    start = time.monotonic()
    started_wall = datetime.now(timezone.utc)

    verbose = args.verbose
    live = {"nl_pending": False, "delta_chars": 0}  # chunk-stream state

    def emit_line(s: str) -> None:
        if live["nl_pending"]:
            print()
            live["nl_pending"] = False
        print(s, flush=True)

    def emit_chunk(s: str) -> None:
        print(s, end="", flush=True)
        live["delta_chars"] += len(s)
        live["nl_pending"] = not s.endswith("\n")

    def stdout_reader(stream) -> None:
        for line in stream:
            text = line.rstrip("\n")
            elapsed = time.monotonic() - start
            try:
                obj = json.loads(text)
            except json.JSONDecodeError:
                obj = None
            with print_lock:
                idx = len(envelopes)
                if not isinstance(obj, dict):
                    envelopes.append({"i": idx, "t_s": round(elapsed, 3), "raw": text})
                    emit_line(text)  # unexpected verbatim output: always show
                    continue
                envelopes.append({"i": idx, "t_s": round(elapsed, 3), "event": obj})
                payload = obj.get("payload")
                if not isinstance(payload, dict):
                    payload = {}
                pt = obj.get("payload_type", "")
                if pt == "run.usage":
                    _usage_from_run_usage(payload, usage)
                else:
                    _walk_usage(obj, "", usage)
                if _mentions_compaction(obj):
                    compaction_indices.append(idx)
                if pt == "run.output.delta":
                    chunk = payload.get("text", "")
                    if isinstance(chunk, str) and chunk:
                        emit_chunk(chunk)
                elif pt == "tool.result":
                    facts = payload.get("correlation_facts", {})
                    if not isinstance(facts, dict):
                        facts = {}
                    emit_line(f"… tool {facts.get('tool_name', '?')} "
                              f"→ {facts.get('outcome', '?')}")
                elif pt.startswith("run.terminal."):
                    if live["delta_chars"] == 0:
                        final = payload.get("text", "")
                        if isinstance(final, str) and final.strip():
                            emit_line(final[:DISPLAY_TRUNCATE])
                    elif live["nl_pending"]:
                        print(flush=True)
                        live["nl_pending"] = False
                elif verbose:
                    raw = text if len(text) <= DISPLAY_TRUNCATE else (
                        text[:DISPLAY_TRUNCATE] + "…(truncated; see events.jsonl)")
                    emit_line(f"# [{pt or 'event'}] {raw}")
                # quiet default: lifecycle/config/status events live in
                # events.jsonl only.

    def stderr_reader(stream) -> None:
        for line in stream:
            text = line.rstrip("\n")
            with print_lock:
                stderr_lines.append(text)
                if verbose:
                    emit_line(f"[stderr] {text}")

    proc = subprocess.Popen(child_argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            text=True, errors="replace", bufsize=1,
                            cwd=str(WORKSPACE), env=env)
    assert proc.stdout and proc.stderr  # pipes requested above
    threads = [threading.Thread(target=stdout_reader, args=(proc.stdout,)),
               threading.Thread(target=stderr_reader, args=(proc.stderr,))]
    for t in threads:
        t.start()
    interrupted = False
    try:
        exit_code: int | None = proc.wait()
    except KeyboardInterrupt:
        interrupted = True
        exit_code = 130
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
    for t in threads:
        t.join()
    end = time.monotonic()

    (run_dir / "events.jsonl").write_text(
        "".join(json.dumps(e) + "\n" for e in envelopes))
    (run_dir / "stderr.log").write_text(
        "".join(line + "\n" for line in stderr_lines))
    # Capture the agent's refactor result before workspace cleanup, for
    # comparison against assets/expected/native.diff. Intent-to-add first so
    # newly created files (e.g. the extracted test module) appear in the
    # diff; build output and gitrs history stay out.
    try:
        subprocess.run(["git", "-C", str(hb_dir), "add", "-N", "--", ".",
                        ":!.history", ":!target"],
                       check=False, capture_output=True, text=True)
        # NOTE: `diff HEAD`, not bare `diff`: gitrs apply stages old-path
        # removals on file-rename sections, so bare diff would miss them.
        diff = subprocess.run(["git", "-C", str(hb_dir), "diff", "HEAD", "--", ".",
                               ":!.history", ":!target"],
                              check=False, capture_output=True, text=True)
        (run_dir / "agent.diff").write_text(diff.stdout if diff.returncode == 0 else "")
        status = subprocess.run(["git", "-C", str(hb_dir), "status", "--short"], check=False,
                                capture_output=True, text=True)
        (run_dir / "agent-status.txt").write_text(
            status.stdout if status.returncode == 0 else "")
    except OSError as e:
        print(f"(warning: could not capture agent diff: {e})", flush=True)
    summary = {
        "demo": "04-live-agent-refactor",
        "mode": mode,
        "model": args.model,
        "session_id": session_id,
        "started_at": started_wall.isoformat(),
        "ended_at": datetime.now(timezone.utc).isoformat(),
        "runtime_s": round(end - start, 1),
        "exit_code": exit_code,
        "interrupted": interrupted,
        "stdout_lines": len(envelopes),
        "json_events": sum(1 for e in envelopes if "event" in e),
        "usage_observed": usage,
        "usage_note": ("from run.usage (+ any token-ish fields on other events); "
                       "empty means no usage was reported — re-mine events.jsonl"),
        "compactions_observed": len(compaction_indices),
        "compaction_event_indices": compaction_indices,
        "key_source": key_source,
        "workspace": str(WORKSPACE),
        "run_dir": str(run_dir),
        "agent_diff": "agent.diff",
        "agent_status": "agent-status.txt",
        "flags": {
            "disable_sandbox": args.disable_sandbox,
            "contributor_mode": args.contributor_mode,
            "approval_mode": args.approval_mode,
            "transport_trace": args.transport_trace,
            "max_model_steps": args.max_model_steps,
        },
    }
    (run_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")

    if live["nl_pending"]:
        print()
    print("==== run summary ====")
    print(f"mode:         {mode}")
    print(f"model:        {args.model}")
    print(f"runtime:      {summary['runtime_s']}s")
    print(f"exit code:    {exit_code} (0=turn complete; correctness NOT implied)")
    print(f"events:       {summary['stdout_lines']} lines ({summary['json_events']} json)")
    print(f"usage:        {json.dumps(usage) if usage else '(none observed — see events.jsonl)'!s}")
    print(f"compactions:  {len(compaction_indices)}")
    print(f"agent diff:   {run_dir / 'agent.diff'}")
    try:
        shutil.rmtree(WORKSPACE)
        print("workspace:    removed")
    except OSError as e:
        print(f"workspace:    cleanup failed ({e}); remove manually")
    print(f"run dir:      {run_dir}")
    return exit_code if exit_code is not None else 1


if __name__ == "__main__":
    sys.exit(main())