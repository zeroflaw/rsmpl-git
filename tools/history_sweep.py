#!/usr/bin/env python3
"""Full-history diffrs sweep (T421): the SPEC T346 recipe per commit.

For every commit in <rev-range> (default HEAD): check out the commit in
an isolated worktree, convert it with the real `diffrs` binary
(`diffrs c~1`, empty-tree rev for the root), round-trip the captured
patch through `applyrs --check` against an isolated pre-change tree
(`git archive c~1 | tar -x`, empty dir for root), then real-apply and
byte-compare against the child tree (`--check` is blind to sweep
over-match, T361).

Unlike `bin/diffrs/tests/history_e2e.rs` (curated pins that fail on any
drift), this runner is exploratory: new compression is success, so it
REPORTS the compressing set instead of asserting it. Failing
conversions become pins in history_e2e.rs.

Usage:
    python3 tools/history_sweep.py [rev-range] [--limit N]

Sem-hunk counts use the anchored diet predicate
`@@ … rule=r` (all RSmPL hunks carry diet `r1…` ids); substring counts inflate
inside test-file string literals (T358 lesson).
"""

from __future__ import annotations

import argparse
import io
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

EMPTY_TREE = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
SEM_PREFIX = "@@ -1,1 @@ rule="


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


def materialize(top: Path, rev: str, dest: Path) -> bool:
    """Unpack `git archive rev` into dest (preserves symlinks/modes)."""
    out = run(["git", "-C", str(top), "archive", rev])
    if out.returncode != 0:
        return False
    try:
        with tarfile.open(fileobj=io.BytesIO(out.stdout)) as tf:
            try:
                tf.extractall(dest, filter="data")
            except TypeError:  # Python < 3.12: no filter argument.
                tf.extractall(dest)
        return True
    except (tarfile.TarError, OSError):
        return False


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


def kind_of(p: Path) -> str | None:
    try:
        st = p.lstat()
    except OSError:
        return None
    import stat

    mode = st.st_mode
    if stat.S_ISLNK(mode):
        return "symlink"
    if stat.S_ISDIR(mode):
        return "dir"
    return "file"


def compare_trees(applied: Path, expect: Path) -> list[str]:
    """Kind + content compare, mirroring history_e2e compare_trees."""
    diffs: list[str] = []
    both = sorted(set(walk(applied)) | set(walk(expect)))
    for rel in both:
        a, e = applied / rel, expect / rel
        ka, ke = kind_of(a), kind_of(e)
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
        else:
            try:
                same = a.read_bytes() == e.read_bytes()
            except OSError:
                same = False
            if not same:
                diffs.append(f"content differs: {rel}")
    return diffs


def check_case(top: Path, wt: Path, tmp: Path, diffrs: Path, applyrs: Path, commit: str, root: bool) -> tuple[int, list[str]]:
    """Convert + round-trip one commit. Returns (sem_count, failures)."""
    short = commit[:7]
    failures: list[str] = []
    if not git_ok(wt, ["checkout", "-qf", commit]):
        return 0, [f"{short}: checkout failed"]
    rev = EMPTY_TREE if root else f"{commit}~1"
    out = run([str(diffrs), "--repo", str(wt), rev])
    if out.returncode != 0:
        return 0, [f"{short}: diffrs {rev} exited {out.returncode}: {out.stderr.decode(errors='replace').strip()}"]
    patch = out.stdout.decode(errors="replace")
    sem = sum(1 for line in patch.splitlines() if is_sem_hunk(line))
    patch_path = tmp / f"{short}.rsmpl"
    patch_path.write_text(patch)

    pre = Path(tempfile.mkdtemp(prefix=f"pre-{short}-"))
    expect_dir: Path | None = None
    try:
        if not root and not materialize(top, f"{commit}~1", pre):
            return sem, [f"{short}: git archive {commit}~1 failed"]

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
        expect_dir = Path(tempfile.mkdtemp(prefix=f"exp-{short}-"))
        if not materialize(top, commit, expect_dir):
            return sem, failures + [f"{short}: git archive {commit} failed"]
        diffs = compare_trees(pre, expect_dir)
        if diffs:
            msg = f"{short}: applied tree differs from {commit}:"
            for d in diffs[:5]:
                msg += f"\n  {d}"
                if len(diffs) > 5:
                    msg += f"\n... and {len(diffs) - 5} more"
            failures.append(msg)
        return sem, failures
    finally:
        shutil.rmtree(pre, ignore_errors=True)
        if expect_dir is not None:
            shutil.rmtree(expect_dir, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("rev_range", nargs="?", default="HEAD", help="rev-list range (default: HEAD)")
    ap.add_argument("--limit", type=int, default=None, metavar="N", help="convert at most N commits (smoke)")
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
    build = run(["cargo", "build", "--bin", "diffrs", "--bin", "applyrs"], cwd=top)
    if build.returncode != 0:
        print(f"error: cargo build failed:\n{build.stderr.decode(errors='replace')}", file=sys.stderr)
        return 2
    diffrs = top / "target" / "debug" / "diffrs"
    applyrs = top / "target" / "debug" / "applyrs"
    # Ensure the empty-tree object (fresh clones may lack it; idempotent).
    run(["git", "-C", str(top), "mktree"], inp=b"")

    out = run(["git", "-C", str(top), "rev-list", "--topo-order", "--reverse", args.rev_range])
    if out.returncode != 0:
        print(f"error: rev-list {args.rev_range} failed", file=sys.stderr)
        return 2
    commits = out.stdout.decode().split()
    if args.limit is not None:
        commits = commits[: args.limit]
    if not commits:
        print("no commits in range", file=sys.stderr)
        return 2

    tmp = Path(tempfile.mkdtemp(prefix="sweep-"))
    wt = tmp / "wt"
    if not git_ok(top, ["worktree", "add", "--detach", str(wt), commits[0]]):
        print("error: worktree add failed", file=sys.stderr)
        shutil.rmtree(tmp, ignore_errors=True)
        return 2
    try:
        total_failures: list[str] = []
        failed: set[str] = set()
        compressing: list[tuple[str, int]] = []
        for i, commit in enumerate(commits):
            # Root = no parent.
            parents = run(["git", "-C", str(top), "rev-list", "--parents", "-n", "1", commit])
            root = len(parents.stdout.decode().split()) == 1
            sem, failures = check_case(top, wt, tmp, diffrs, applyrs, commit, root)
            if failures:
                failed.add(commit[:7])
            total_failures.extend(failures)
            if sem:
                compressing.append((commit[:7], sem))
            if (i + 1) % 50 == 0:
                print(f"... {i + 1}/{len(commits)}", file=sys.stderr)
        print(f"head {head_full} range {args.rev_range} commits {len(commits)} tip {commits[-1][:7]} root {commits[0][:7]}")
        head_short = head_full[:7] if len(head_full) >= 7 else head_full
        print(f"convert {len(commits) - len(failed)}/{len(commits)} (head {head_short})")
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
        return 1 if total_failures else 0
    finally:
        run(["git", "-C", str(top), "worktree", "remove", "--force", str(wt)])
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
