#!/usr/bin/env python3
"""Directive-inventory coverage gate (WI-24 slice 1).

Source of truth: the parser itself —
  crates/rsmpl_patch/src/directives.rs (KNOWN_KV_KEYS, BARE_FLAGS, ALL)
  crates/rsmpl_patch/src/model.rs (MetavarKind::parse arms).
SPEC.md must cover every key/kind; the tables stay hand-written, only
the coverage is machine-checked (WI-24 boundary: prose hand-owned).

Usage:
  python3 tools/gen_directives.py --check   # exit 0 in sync, 2 on drift
Bare run defaults to --check. No --write: coverage is fixed by
hand-editing SPEC (one flag only, per the minimal-CLI rule).
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIRECTIVES = ROOT / "crates" / "rsmpl_patch" / "src" / "directives.rs"
MODEL = ROOT / "crates" / "rsmpl_patch" / "src" / "model.rs"
SPEC = ROOT / "SPEC.md"


def parse_code():
    src = DIRECTIVES.read_text()

    def str_list(name: str):
        m = re.search(rf"{name}[^=]*=\s*&\[(.*?)\]", src, re.S)
        if not m:
            raise SystemExit(f"gen_directives: {name} not found")
        return re.findall(r'"([^"]+)"', m.group(1))

    keys = str_list("KNOWN_KV_KEYS")
    bare = str_list("BARE_FLAGS")
    all_list = str_list("const ALL")
    model = MODEL.read_text()
    kinds = re.findall(r'"([a-z_]+)"\s*=>\s*Some\(Self::', model)
    if not kinds:
        raise SystemExit("gen_directives: MetavarKind arms not found")
    return keys, bare, all_list, kinds


def spec_section(text: str, start: str, stops) -> str:
    i = text.find(start)
    if i < 0:
        raise SystemExit(f"gen_directives: SPEC section {start!r} not found")
    tail = text[i + len(start):]
    end = len(tail)
    for s in stops:
        j = tail.find(s)
        if 0 <= j < end:
            end = j
    return tail[:end]


def main(argv) -> int:
    args = argv or ["--check"]
    if args != ["--check"]:
        print("gen_directives: usage: `python3 tools/gen_directives.py --check`")
        return 2
    keys, bare, all_list, kinds = parse_code()
    spec = SPEC.read_text()
    s93 = spec_section(spec, "### 9.3 Core directives", ["### 9.4", "\n## 10"])
    s151 = spec_section(
        spec, "### 15.1 Supported metavariable kinds", ["### 15.2"]
    )

    missing_keys = [k for k in keys if f"# {k}" not in s93]
    missing_bare = [b for b in bare if f"# {b}" not in s93]
    missing_kinds = [k for k in kinds if f"`{k}`" not in s151]
    missing_rejected = [
        d
        for d in ["# edition", "# format_scope"]
        if d not in spec
    ]
    union = sorted(set(keys) | set(bare))
    stale_all = sorted(set(all_list) - set(union))
    missing_all = sorted(set(union) - set(all_list))

    problems = []
    if missing_keys:
        problems.append(f"keys={missing_keys}")
    if missing_bare:
        problems.append(f"bare={missing_bare}")
    if missing_kinds:
        problems.append(f"kinds={missing_kinds}")
    if missing_rejected:
        problems.append(f"rejected={missing_rejected}")
    if stale_all or missing_all:
        problems.append(f"ALL_union_stale={stale_all} ALL_union_missing={missing_all}")
    if problems:
        print(f"gen_directives: DRIFT {'; '.join(problems)}")
        print("gen_directives: cover the missing key/kind in SPEC 9.3/15.1")
        return 2
    print(
        f"gen_directives: in sync "
        f"({len(keys)} keys, {len(bare)} bare, {len(kinds)} kinds)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
