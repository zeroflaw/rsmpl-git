#!/usr/bin/env python3
"""Directive-inventory coverage gate + generated inventory blocks (WI-9).

Source of truth: the parser itself —
  crates/rsmpl_patch/src/directives.rs (KNOWN_KV_KEYS, BARE_FLAGS, ALL)
  crates/rsmpl_patch/src/model.rs (MetavarKind::parse arms).
SPEC.md 9.3/15.1 must cover every key/kind (coverage checked); the
inventory blocks between the `generated: directives` markers in SPEC.md,
AGENTS.md, and skills/gitrs/SKILL.md plus the `contract-directives`
one-liner at the SPEC head are generated from the same extractors
(`--write` rewrites them; `--check` fails on drift). Meaning prose stays
hand-written outside the markers — tables and enumerations only.

Usage:
  python3 tools/gen_directives.py --check   # exit 0 in sync, 2 on drift
  python3 tools/gen_directives.py --write   # rewrite the doc blocks
Bare run defaults to --check.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIRECTIVES = ROOT / "crates" / "rsmpl_patch" / "src" / "directives.rs"
MODEL = ROOT / "crates" / "rsmpl_patch" / "src" / "model.rs"
SPEC = ROOT / "SPEC.md"
AGENTS = ROOT / "AGENTS.md"
SKILL = ROOT / "skills" / "gitrs" / "SKILL.md"

START = "<!-- generated: directives start -- do not hand-edit; run `python3 tools/gen_directives.py --write` -->"
END = "<!-- generated: directives end -->"
CSTART = "<!-- generated: contract-directives start -- do not hand-edit; run `python3 tools/gen_directives.py --write` -->"
CEND = "<!-- generated: contract-directives end -->"


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


def render(keys, bare, kinds) -> str:
    return (
        f"{START}\n"
        f"Directive inventory (code is the source: `directives.rs` + `model.rs`): "
        f"{len(keys)} keys, {len(bare)} bare flags, {len(kinds)} kinds.\n"
        f"Keys: {', '.join(keys)}.\n"
        f"Bare: {', '.join(bare)}.\n"
        f"Kinds: {', '.join(kinds)}.\n"
        f"{END}\n"
    )


def render_contract(keys, bare, kinds) -> str:
    return (
        f"{CSTART}\n"
        f"Directives: {len(keys)} keys, {len(bare)} bare flags, {len(kinds)} kinds "
        f"(SPEC 9.3; generated block).\n"
        f"{CEND}\n"
    )


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


def block(text: str, start: str, end: str, where: str) -> str:
    i = text.find(start)
    j = text.find(end)
    if i < 0 or j < 0 or j < i:
        raise SystemExit(f"gen_directives: markers not found in {where}")
    return text[i : j + len(end)] + "\n"


def check_coverage(spec: str, keys, bare, all_list, kinds):
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
    return problems


def main(argv) -> int:
    args = argv or ["--check"]
    keys, bare, all_list, kinds = parse_code()
    spec = SPEC.read_text()
    agents = AGENTS.read_text()
    skill = SKILL.read_text()
    expected = render(keys, bare, kinds)
    expected_c = render_contract(keys, bare, kinds)
    targets = [
        (spec, START, END, "SPEC.md directives"),
        (agents, START, END, "AGENTS.md directives"),
        (skill, START, END, "SKILL.md directives"),
    ]
    if "--write" in args:
        changed = []
        for path, s, e in [(SPEC, START, END), (AGENTS, START, END), (SKILL, START, END)]:
            text = path.read_text()
            cur = block(text, s, e, str(path))
            if cur != expected:
                path.write_text(text.replace(cur, expected))
                changed.append(path.name)
        text = SPEC.read_text()
        cur_c = block(text, CSTART, CEND, "SPEC.md contract-directives")
        if cur_c != expected_c:
            SPEC.write_text(text.replace(cur_c, expected_c))
            changed.append("SPEC.md(contract)")
        if changed:
            print(f"gen_directives: wrote ({', '.join(sorted(set(changed)))})")
        else:
            print(
                f"gen_directives: in sync "
                f"({len(keys)} keys, {len(bare)} bare, {len(kinds)} kinds)"
            )
        return 0
    if args != ["--check"]:
        print("gen_directives: usage: `python3 tools/gen_directives.py [--check|--write]`")
        return 2
    problems = check_coverage(spec, keys, bare, all_list, kinds)
    for text, s, e, where in targets:
        if block(text, s, e, where) != expected:
            problems.append(f"stale-block:{where}")
    if block(spec, CSTART, CEND, "SPEC.md contract-directives") != expected_c:
        problems.append("stale-block:SPEC.md contract-directives")
    if problems:
        print(f"gen_directives: DRIFT {'; '.join(problems)}")
        print("gen_directives: run `python3 tools/gen_directives.py --write`")
        return 2
    print(
        f"gen_directives: in sync "
        f"({len(keys)} keys, {len(bare)} bare, {len(kinds)} kinds)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
