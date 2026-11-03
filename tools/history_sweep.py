#!/usr/bin/env python3
"""Full-history diffrs sweep (T421): the SPEC T346 recipe per commit.

For every commit in <rev-range> (default HEAD): check out the commit in
an isolated worktree, convert it with the real `diffrs` binary
(`diffrs c~1`, empty-tree rev for the root), round-trip the captured
patch through `applyrs --check` against an isolated pre-change tree,
then real-apply and byte-compare against the child tree (`--check` is
blind to sweep over-match, T361).

Unlike `bin/diffrs/tests/history_e2e.rs` (curated pins that fail on any
drift), this runner is exploratory: new compression is success, so it
REPORTS the compressing set instead of asserting it. Failing
conversions become pins in history_e2e.rs.

Performance notes: the sweep uses `--release` builds of
gitrs/applyrs/diffrs (rebuilt + freshness-verified before use),
resolves all parents with one `rev-list --parents` call, runs commits
in parallel across contiguous chunks (one isolated worktree pair per
worker, so checkouts stay incremental), and byte-compares the applied
pre-change worktree directly against the child worktree (no second
full-tree materialize).

Usage:
    python3 tools/history_sweep.py [rev-range] [--limit N] [--jobs N]
    python3 tools/history_sweep.py --self-test

  With no rev-range the sweep processes the entire HEAD history
  (`rev-list HEAD`): per commit, convert with the real `diffrs`
  (`diffrs c~1`, empty-tree rev for the root), round-trip through
  `applyrs --check`, then real-apply and byte-compare against the child
  tree. Exit 0 iff every commit converts and re-applies; any failure
  prints the commit hash, the failing stage, and the first divergence
  and exits 2. Compression (semantic-hunk counts per commit and in
  total) is printed as a report, never as a pass/fail: there is no
  floor, no pinned window, and no floor file (D19 smoke, not a gate).
  `--self-test` builds a synthetic three-commit repo and checks the
  sweep predicates (compressing + ordinary arms, plus a failure arm:
  a garbage patch must fail `applyrs --check`).

  No CI wiring (forbidden in this repo) — run it by hand beside
  example-benches/run.sh.

Sem-hunk counts use the anchored diet predicate
`@@ … rule=r` (all RSmPL hunks carry diet `r1…` ids); substring counts inflate
inside test-file string literals (T358 lesson).
"""

from __future__ import annotations

import argparse
import concurrent.futures
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

EMPTY_TREE = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
BINS = ("gitrs", "applyrs", "diffrs")
# Serializes worktree add/remove only (cheap insurance against
# .git/worktrees races); all per-commit work runs fully parallel.
_wt_lock = threading.Lock()


def is_sem_hunk(line: str) -> bool:
    """Anchored semantic-hunk test (T358-safe + WI-1e diet: diff content
    lines never start with `@@`); diet rule ids are `r1…`, so any
    `@@ … rule=r` header is semantic — dummy `# sweep` hunks plus
    ranged move/partial-delete hunks alike."""
    return line.startswith("@@ ") and "rule=r" in line


def run(cmd: list[str], cwd: Path | None = None, inp: bytes | None = None) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(cmd, cwd=cwd, input=inp, stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def git_ok(top: Path, args: list[str]) -> bool:
    return run(["git", "-C", str(top)] + args).returncode == 0


def toplevel() -> Path | None:
    out = run(["git", "-C", str(Path(__file__).resolve().parent.parent), "rev-parse", "--show-toplevel"])
    if out.returncode != 0:
        return None
    return Path(out.stdout.decode().strip())


def ensure_release_bins(top: Path) -> dict[str, Path] | None:
    """Rebuild + verify the release binaries before use.

    A successful `cargo build --release` is the freshness oracle: cargo
    fingerprints already prove each binary is up to date with its
    transitive sources (and cargo correctly skips relinking binaries
    unaffected by e.g. a `Cargo.lock` touch after a dep add, leaving
    their mtime older than the lockfile — a raw mtime comparison here
    false-aborts as "stale", so there is deliberately no mtime check).
    """
    build = run(["cargo", "build", "--release", "--bin", "gitrs", "--bin", "applyrs", "--bin", "diffrs"], cwd=top)
    if build.returncode != 0:
        print(f"error: cargo build --release failed:\n{build.stderr.decode(errors='replace')}", file=sys.stderr)
        return None
    bins: dict[str, Path] = {}
    for name in BINS:
        b = top / "target" / "release" / name
        if not b.is_file() or not os.access(b, os.X_OK):
            print(f"error: release binary missing/not executable: {b}", file=sys.stderr)
            return None
        bins[name] = b
    return bins


def walk(root: Path) -> list[Path]:
    found: list[Path] = []
    stack = [Path()]
    while stack:
        rel = stack.pop()
        try:
            entries = list((root / rel).iterdir())
        except OSError:
            continue
        for e in entries:
            r = rel / e.name
            if len(r.parts) == 1 and r.parts[0] in (".history", ".git"):
                continue
            if e.is_dir() and not e.is_symlink():
                stack.append(r)
            found.append(r)
    return sorted(found)


def info(p: Path) -> tuple[str | None, int]:
    """(kind, size): one lstat serves the kind check and the size pre-check."""
    try:
        st = p.lstat()
    except OSError:
        return None, 0
    mode = st.st_mode
    if stat.S_ISLNK(mode):
        return "symlink", 0
    if stat.S_ISDIR(mode):
        return "dir", 0
    return "file", st.st_size


def compare_trees(applied: Path, expect: Path) -> list[str]:
    """Kind + content compare, mirroring history_e2e compare_trees."""
    diffs: list[str] = []
    both = sorted(set(walk(applied)) | set(walk(expect)))
    for rel in both:
        a, e = applied / rel, expect / rel
        ka, sa = info(a)
        ke, se = info(e)
        if ka is None and ke is None:
            continue
        if ka is None:
            diffs.append(f"missing in applied: {rel}")
        elif ke is None:
            diffs.append(f"extra in applied: {rel}")
        elif ka != ke:
            diffs.append(f"kind differs ({ka} vs {ke}): {rel}")
        elif ka == "dir":
            pass
        elif ka == "symlink":
            try:
                same = a.readlink() == e.readlink()
            except OSError:
                same = False
            if not same:
                diffs.append(f"symlink target differs: {rel}")
        elif sa != se:
            diffs.append(f"content differs: {rel}")
        else:
            try:
                same = a.read_bytes() == e.read_bytes()
            except OSError:
                same = False
            if not same:
                diffs.append(f"content differs: {rel}")
    return diffs


def wipe(d: Path) -> None:
    """Empty a worktree (root-commit pre-state), keeping its `.git` pointer."""
    for e in d.iterdir():
        if e.name == ".git":
            continue
        if e.is_dir() and not e.is_symlink():
            shutil.rmtree(e, ignore_errors=True)
        else:
            e.unlink(missing_ok=True)


def check_case(
    wt: Path, pre: Path, patch_path: Path, diffrs: Path, applyrs: Path, commit: str, parent: str | None
) -> tuple[int, list[str]]:
    """Convert + round-trip one commit. Returns (sem_count, failures)."""
    short = commit[:7]
    failures: list[str] = []
    if not git_ok(wt, ["checkout", "-qf", commit]):
        return 0, [f"{short}: checkout failed"]
    if parent is None:
        wipe(pre)
        rev = EMPTY_TREE
    else:
        if not git_ok(pre, ["checkout", "-qf", parent]):
            return 0, [f"{short}: pre-checkout {parent[:7]} failed"]
        rev = f"{commit}~1"
    out = run([str(diffrs), "--repo", str(wt), rev])
    if out.returncode != 0:
        return 0, [f"{short}: diffrs {rev} exited {out.returncode}: {out.stderr.decode(errors='replace').strip()}"]
    patch = out.stdout.decode(errors="replace")
    sem = sum(1 for line in patch.splitlines() if is_sem_hunk(line))
    patch_path.write_text(patch)

    checked = run([str(applyrs), "--file", str(patch_path), "--check", "--repo", str(pre)])
    if checked.returncode != 0:
        failures.append(
            f"{short}: applyrs --check exited {checked.returncode}: "
            f"{checked.stderr.decode(errors='replace').strip()}"
        )
    applied = run([str(applyrs), "--file", str(patch_path), "--repo", str(pre)])
    if applied.returncode != 0:
        failures.append(
            f"{short}: applyrs apply exited {applied.returncode}: "
            f"{applied.stderr.decode(errors='replace').strip()}"
        )
        return sem, failures
    # The child worktree `wt` already holds the post-change tree, so it
    # doubles as the expected tree (no second full-tree materialize).
    diffs = compare_trees(pre, wt)
    if diffs:
        msg = f"{short}: applied tree differs from {commit}:"
        for d in diffs[:5]:
            msg += f"\n  {d}"
            if len(diffs) > 5:
                msg += f"\n... and {len(diffs) - 5} more"
        failures.append(msg)
    return sem, failures


def sweep_chunk(
    top: Path,
    tmp: Path,
    idx: int,
    chunk: list[str],
    parents_of: dict[str, str | None],
    diffrs: Path,
    applyrs: Path,
    progress: list,
    total: int,
) -> list[tuple[str, int, list[str]]]:
    """Sweep one contiguous chunk with a private worktree pair."""
    wt = tmp / f"wt-{idx}"
    pre = tmp / f"pre-{idx}"
    with _wt_lock:
        ok_wt = git_ok(top, ["worktree", "add", "--detach", str(wt), chunk[0]])
        ok_pre = ok_wt and git_ok(top, ["worktree", "add", "--detach", str(pre), chunk[0]])
    if not (ok_wt and ok_pre):
        with _wt_lock:
            run(["git", "-C", str(top), "worktree", "remove", "--force", str(wt)])
            run(["git", "-C", str(top), "worktree", "remove", "--force", str(pre)])
        return [(c[:7], 0, [f"{c[:7]}: worktree add failed"]) for c in chunk]
    try:
        results: list[tuple[str, int, list[str]]] = []
        patch_path = tmp / f"w{idx}.rsmpl"
        for commit in chunk:
            sem, failures = check_case(wt, pre, patch_path, diffrs, applyrs, commit, parents_of[commit])
            results.append((commit[:7], sem, failures))
            with progress[0]:
                progress[1] += 1
                if progress[1] % 50 == 0:
                    print(f"... {progress[1]}/{total}", file=sys.stderr)
        return results
    finally:
        with _wt_lock:
            run(["git", "-C", str(top), "worktree", "remove", "--force", str(wt)])
            run(["git", "-C", str(top), "worktree", "remove", "--force", str(pre)])


def list_commits(top: Path, rev_range: str, limit: int | None) -> tuple[list[str], dict[str, str | None]] | None:
    """rev-list the range in topo-reverse order with first parents."""
    out = run(["git", "-C", str(top), "rev-list", "--topo-order", "--reverse", "--parents", rev_range])
    if out.returncode != 0:
        print(f"error: rev-list {rev_range} failed", file=sys.stderr)
        return None
    commits: list[str] = []
    parents_of: dict[str, str | None] = {}
    for line in out.stdout.decode().splitlines():
        toks = line.split()
        if not toks:
            continue
        commits.append(toks[0])
        parents_of[toks[0]] = toks[1] if len(toks) > 1 else None
    if limit is not None:
        commits = commits[:limit]
    return commits, parents_of


def run_sweep(
    top: Path,
    commits: list[str],
    parents_of: dict[str, str | None],
    diffrs: Path,
    applyrs: Path,
    jobs: int,
) -> list[tuple[str, int, list[str]]]:
    """Fan the commits out over worktree pairs; returns per-commit results."""
    tmp = Path(tempfile.mkdtemp(prefix="sweep-"))
    try:
        jobs = max(1, min(jobs, len(commits)))
        base, extra = divmod(len(commits), jobs)
        chunks: list[tuple[int, list[str]]] = []
        s = 0
        for i in range(jobs):
            e = s + base + (1 if i < extra else 0)
            chunks.append((i, commits[s:e]))
            s = e
        progress: list = [threading.Lock(), 0]
        total = len(commits)
        with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as ex:
            futs = [
                ex.submit(sweep_chunk, top, tmp, i, chunk, parents_of, diffrs, applyrs, progress, total)
                for i, chunk in chunks
            ]
            ordered = [f.result() for f in futs]
        flat: list[tuple[str, int, list[str]]] = []
        for results in ordered:
            flat.extend(results)
        return flat
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def run_self_test(top: Path, bins: dict[str, Path]) -> int:
    """Synthetic smoke check: one compressing rename, one ordinary, one failure arm."""
    diffrs, applyrs = bins["diffrs"], bins["applyrs"]
    tmp = Path(tempfile.mkdtemp(prefix="sweep-selftest-"))
    try:
        repo = tmp / "repo"
        repo.mkdir()
        run(["git", "-C", str(repo), "init", "-q"])
        run(["git", "-C", str(repo), "config", "user.email", "t@t"])
        run(["git", "-C", str(repo), "config", "user.name", "t"])
        (repo / "a.rs").write_text("fn foo() -> i32 {\n    1\n}\n\nfn main() {\n    let x = foo();\n}\n")
        run(["git", "-C", str(repo), "add", "-A"])
        run(["git", "-C", str(repo), "commit", "-qm", "base"])
        (repo / "a.rs").write_text("fn bar() -> i32 {\n    1\n}\n\nfn main() {\n    let x = bar();\n}\n")
        run(["git", "-C", str(repo), "add", "-A"])
        run(["git", "-C", str(repo), "commit", "-qm", "rename"])
        (repo / "a.rs").write_text("fn bar() -> i32 {\n    1\n}\n\n// trailing comment\nfn main() {\n    let x = bar();\n}\n")
        run(["git", "-C", str(repo), "add", "-A"])
        run(["git", "-C", str(repo), "commit", "-qm", "comment"])
        out = run(["git", "-C", str(repo), "rev-list", "--topo-order", "--reverse", "--parents", "HEAD"])
        commits: list[str] = []
        parents_of: dict[str, str | None] = {}
        for line in out.stdout.decode().splitlines():
            toks = line.split()
            if toks:
                commits.append(toks[0])
                parents_of[toks[0]] = toks[1] if len(toks) > 1 else None
        # commits[0] is the root (empty pre-image); sweep the two children.
        results = run_sweep(repo, commits[1:], parents_of, diffrs, applyrs, 1)
        sems = [sem for _, sem, _ in results]
        fails = [f for _, _, fl in results for f in fl]
        if fails:
            print(f"SELF-TEST FAIL: sweep failures: {fails}", file=sys.stderr)
            return 2
        if len(sems) != 2 or not (sems[0] >= 1 and sems[1] == 0):
            print(f"SELF-TEST FAIL: expected [compressing, ordinary], got sems={sems}", file=sys.stderr)
            return 2
        # Failure arm: a garbage patch must fail `applyrs --check`, proving
        # the smoke's check stage can fail (an unappliable raise fails loud).
        bad = tmp / "bad.rsmpl"
        bad.write_text("this is not a patch\n")
        checked = run([str(applyrs), "--file", str(bad), "--check", "--repo", str(repo)])
        if checked.returncode == 0:
            print("SELF-TEST FAIL: garbage patch passed --check", file=sys.stderr)
            return 2
        print(f"SELF-TEST PASS: sems={sems} check_rejects_garbage")
        return 0
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("rev_range", nargs="?", default="HEAD", help="rev-list range (default: HEAD = whole history)")
    ap.add_argument("--limit", type=int, default=None, metavar="N", help="convert at most N commits (smoke)")
    ap.add_argument(
        "-j", "--jobs", type=int, default=os.cpu_count() or 4, metavar="N",
        help="parallel workers (default: cpu count)",
    )
    ap.add_argument(
        "--self-test", action="store_true",
        help="synthetic smoke check (compressing + ordinary + failure arms)",
    )
    args = ap.parse_args()

    top = toplevel()
    if top is None:
        print("error: not a git checkout", file=sys.stderr)
        return 2
    # T439: capture the swept HEAD so the convert denominator is
    # attributable (loops drifted +1 against the "same" HEAD via
    # transient resets; the swept list is fixed, the hash pins it).
    head_out = run(["git", "-C", str(top), "rev-parse", "HEAD"])
    head_full = head_out.stdout.decode().strip() if head_out.returncode == 0 else "unknown"
    bins = ensure_release_bins(top)
    if bins is None:
        return 2
    diffrs, applyrs = bins["diffrs"], bins["applyrs"]
    # Ensure the empty-tree object (fresh clones may lack it; idempotent).
    run(["git", "-C", str(top), "mktree"], inp=b"")

    if args.self_test:
        return run_self_test(top, bins)

    listed = list_commits(top, args.rev_range, args.limit)
    if listed is None:
        return 2
    commits, parents_of = listed
    if not commits:
        print("no commits in range", file=sys.stderr)
        return 2

    ordered = run_sweep(top, commits, parents_of, diffrs, applyrs, args.jobs)
    total_failures: list[str] = []
    failed: set[str] = set()
    compressing: list[tuple[str, int]] = []
    for short, sem, failures in ordered:
        if failures:
            failed.add(short)
        total_failures.extend(failures)
        if sem:
            compressing.append((short, sem))
    print(f"head {head_full} range {args.rev_range} commits {len(commits)} tip {commits[-1][:7]} root {commits[0][:7]}")
    head_short = head_full[:7] if len(head_full) >= 7 else head_full
    total_sem = sum(sem for _, sem in compressing)
    print(f"convert {len(commits) - len(failed)}/{len(commits)} failures {len(total_failures)} compressing {len(compressing)} semantic_hunks {total_sem} (head {head_short})")
    if total_failures:
        print("failures:")
        for f in total_failures:
            print(f"  {f}")
    if compressing:
        print("compressing:")
        for short, sem in sorted(compressing):
            print(f"  {short} sem={sem}")
    else:
        print("compressing: none")
    return 2 if total_failures else 0


if __name__ == "__main__":
    sys.exit(main())
