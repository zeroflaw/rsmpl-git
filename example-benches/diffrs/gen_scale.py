#!/usr/bin/env python3
"""Deterministic generator for the diffrs raising scale benches (WI-21).

Regenerates the `repo/` + `after/` corpora for:
  49-raise-wide: fn rename + 200 anchored call sites across 20 files
  50-raise-deep: 300-item file with 50 mixed hunks (partition worst case)

No dependencies beyond the stdlib; no randomness (fixed content, so
regeneration is byte-identical). `native.patch` / `sem.patch` are NOT
written here — they are produced by `example-benches/run.sh` like every
other bench (freshness + fixed-point tests pin them).

Usage:
  python3 example-benches/diffrs/gen_scale.py           # regenerate in place
  python3 example-benches/diffrs/gen_scale.py --check   # exit 2 naming drift
"""

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

WIDE_FILES = 20
WIDE_SITES_PER_FILE = 10

DEEP_ITEMS = 300
DEEP_EVERY = 6  # every 6th item edited -> 50 hunks


def wide_repo():
    files = {
        "def.rs": "pub fn old_name(x: i32) -> i32 {\n    x + 1\n}\n",
    }
    for i in range(WIDE_FILES):
        lines = ["use crate::def::old_name;"]
        for k in range(WIDE_SITES_PER_FILE):
            n = i * WIDE_SITES_PER_FILE + k
            lines.append(f"fn c{n}() -> i32 {{")
            lines.append(f"    old_name({n})")
            lines.append("}")
        files[f"u{i:02d}.rs"] = "\n".join(lines) + "\n"
    return files


def wide_after():
    return {
        path: content.replace("old_name", "new_name")
        for path, content in wide_repo().items()
    }


def deep_fn(i, mode):
    """One item of the deep corpus. Mode selects the post-state edit."""
    if mode == "call":
        return (
            f"fn f{i:03}(x: i32) -> i32 {{\n"
            f"    let v{i:03} = x + {i};\n"
            f"    bar(v{i:03})\n"
            "}\n"
        )
    if mode == "let":
        return (
            f"fn f{i:03}(x: i32) -> i32 {{\n"
            f"    let v{i:03} = x + {i + 1000};\n"
            f"    foo(v{i:03})\n"
            "}\n"
        )
    if mode == "comment":
        return (
            f"fn f{i:03}(x: i32) -> i32 {{ // tweaked {i}\n"
            f"    let v{i:03} = x + {i};\n"
            f"    foo(v{i:03})\n"
            "}\n"
        )
    return (
        f"fn f{i:03}(x: i32) -> i32 {{\n"
        f"    let v{i:03} = x + {i};\n"
        f"    foo(v{i:03})\n"
        "}\n"
    )


def deep_repo():
    return {"src/lib.rs": "".join(deep_fn(i, "") for i in range(DEEP_ITEMS))}


def deep_after():
    kinds = ("call", "let", "comment")
    parts = []
    for i in range(DEEP_ITEMS):
        if i % DEEP_EVERY == 0:
            parts.append(deep_fn(i, kinds[(i // DEEP_EVERY) % 3]))
        else:
            parts.append(deep_fn(i, ""))
    return {"src/lib.rs": "".join(parts)}


BENCHES = {
    "49-raise-wide": (wide_repo, wide_after),
    "50-raise-deep": (deep_repo, deep_after),
}


def generate(root):
    for name, (repo_fn, after_fn) in BENCHES.items():
        for side, fn in (("repo", repo_fn), ("after", after_fn)):
            for rel, content in fn().items():
                path = os.path.join(root, name, side, rel)
                os.makedirs(os.path.dirname(path), exist_ok=True)
                with open(path, "w") as f:
                    f.write(content)


def check(root):
    drift = []
    for name, (repo_fn, after_fn) in BENCHES.items():
        for side, fn in (("repo", repo_fn), ("after", after_fn)):
            for rel, content in fn().items():
                path = os.path.join(root, name, side, rel)
                try:
                    with open(path) as f:
                        if f.read() != content:
                            drift.append(path)
                except OSError:
                    drift.append(path + " (missing)")
    # No extra files beyond the generated set.
    for name in BENCHES:
        for side in ("repo", "after"):
            base = os.path.join(root, name, side)
            for dirpath, _, filenames in os.walk(base):
                for fn in filenames:
                    full = os.path.join(dirpath, fn)
                    rel = os.path.relpath(full, base)
                    want = BENCHES[name][0 if side == "repo" else 1]()
                    if rel not in want:
                        drift.append(full + " (untracked)")
    return drift


def main(argv):
    if argv == ["--check"]:
        drift = check(HERE)
        if drift:
            print("gen_scale.py drift:")
            for path in drift:
                print(f"  {path}")
            return 2
        print("gen_scale.py: corpora match the generator")
        return 0
    if argv:
        print(f"usage: gen_scale.py [--check]", file=sys.stderr)
        return 2
    generate(HERE)
    print("gen_scale.py: regenerated 49-raise-wide + 50-raise-deep")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
