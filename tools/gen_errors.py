#!/usr/bin/env python3
"""Error-code inventory coverage gate + generated inventory blocks (WI-9).

Source of truth: the engine itself —
  crates/rsmpl_patch/src/errors.rs (ErrorCode::as_str arms).
SPEC.md section 25 plus the AGENTS.md / skills/gitrs/SKILL.md
"Errors: code -> fix" tables must cover every code (coverage checked);
the inventory blocks between the `generated: errors` markers in SPEC.md,
AGENTS.md, and SKILL.md plus the `contract-errors` one-liner at the SPEC
head are generated from the same extractor (`--write` rewrites them;
`--check` fails on drift). Fix prose stays hand-written outside the
markers — tables and enumerations only.

Usage:
  python3 tools/gen_errors.py --check   # exit 0 in sync, 2 on drift
  python3 tools/gen_errors.py --write   # rewrite the doc blocks
Bare run defaults to --check.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ERRORS = ROOT / "crates" / "rsmpl_patch" / "src" / "errors.rs"
SPEC = ROOT / "SPEC.md"
AGENTS = ROOT / "AGENTS.md"
SKILL = ROOT / "skills" / "gitrs" / "SKILL.md"

START = "<!-- generated: errors start -- do not hand-edit; run `python3 tools/gen_errors.py --write` -->"
END = "<!-- generated: errors end -->"
CSTART = "<!-- generated: contract-errors start -- do not hand-edit; run `python3 tools/gen_errors.py --write` -->"
CEND = "<!-- generated: contract-errors end -->"


def parse_codes():
    src = ERRORS.read_text()
    codes = re.findall(r"Self::\w+ => \"([A-Z_]+)\"", src)
    codes = [c for c in codes if c.endswith("CONFLICT")]
    if not codes:
        raise SystemExit("gen_errors: ErrorCode::as_str arms not found")
    if len(set(codes)) != len(codes):
        raise SystemExit(f"gen_errors: duplicate codes: {codes}")
    return codes


def render(codes) -> str:
    return (
        f"{START}\n"
        f"Error-code inventory (code is the source: `errors.rs`): "
        f"{len(codes)} codes.\n"
        f"Codes: {', '.join(codes)}.\n"
        f"{END}\n"
    )


def render_contract(codes) -> str:
    return (
        f"{CSTART}\n"
        f"Errors: {len(codes)} codes (SPEC 25; generated block).\n"
        f"{CEND}\n"
    )


def block(text: str, start: str, end: str, where: str) -> str:
    i = text.find(start)
    j = text.find(end)
    if i < 0 or j < 0 or j < i:
        raise SystemExit(f"gen_errors: markers not found in {where}")
    return text[i : j + len(end)] + "\n"


def main(argv) -> int:
    args = argv or ["--check"]
    codes = parse_codes()
    spec = SPEC.read_text()
    agents = AGENTS.read_text()
    skill = SKILL.read_text()
    expected = render(codes)
    expected_c = render_contract(codes)
    targets = [
        (spec, "SPEC.md errors"),
        (agents, "AGENTS.md errors"),
        (skill, "SKILL.md errors"),
    ]
    if "--write" in args:
        changed = []
        for path in [SPEC, AGENTS, SKILL]:
            text = path.read_text()
            cur = block(text, START, END, str(path))
            if cur != expected:
                path.write_text(text.replace(cur, expected))
                changed.append(path.name)
        text = SPEC.read_text()
        cur_c = block(text, CSTART, CEND, "SPEC.md contract-errors")
        if cur_c != expected_c:
            SPEC.write_text(text.replace(cur_c, expected_c))
            changed.append("SPEC.md(contract)")
        if changed:
            print(f"gen_errors: wrote ({', '.join(sorted(set(changed)))})")
        else:
            print(f"gen_errors: in sync ({len(codes)} codes)")
        return 0
    if args != ["--check"]:
        print("gen_errors: usage: `python3 tools/gen_errors.py [--check|--write]`")
        return 2
    problems = []
    missing_spec = [c for c in codes if c not in spec]
    missing_agents = [c for c in codes if c not in agents]
    missing_skill = [c for c in codes if c not in skill]
    if missing_spec:
        problems.append(f"SPEC-25={missing_spec}")
    if missing_agents:
        problems.append(f"AGENTS={missing_agents}")
    if missing_skill:
        problems.append(f"SKILL={missing_skill}")
    for text, where in targets:
        if block(text, START, END, where) != expected:
            problems.append(f"stale-block:{where}")
    if block(spec, CSTART, CEND, "SPEC.md contract-errors") != expected_c:
        problems.append("stale-block:SPEC.md contract-errors")
    if problems:
        print(f"gen_errors: DRIFT {'; '.join(problems)}")
        print("gen_errors: run `python3 tools/gen_errors.py --write`")
        return 2
    print(f"gen_errors: in sync ({len(codes)} codes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
