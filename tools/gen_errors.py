#!/usr/bin/env python3
"""Error-code inventory coverage gate (WI-24 slice 2).

Source of truth: the engine itself —
  crates/rsmpl_patch/src/errors.rs (ErrorCode::as_str arms).
SPEC.md section 25 plus the AGENTS.md / skills/gitrs/SKILL.md
"Errors: code -> fix" tables must cover every code; the tables stay
hand-written, only the coverage is machine-checked (WI-24 boundary:
prose hand-owned, inventory machine-checked).

Usage:
  python3 tools/gen_errors.py --check   # exit 0 in sync, 2 on drift
Bare run defaults to --check. No --write: coverage is fixed by
hand-editing the tables (one flag only, per the minimal-CLI rule).
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ERRORS = ROOT / "crates" / "rsmpl_patch" / "src" / "errors.rs"
SPEC = ROOT / "SPEC.md"
AGENTS = ROOT / "AGENTS.md"
SKILL = ROOT / "skills" / "gitrs" / "SKILL.md"


def parse_codes():
    src = ERRORS.read_text()
    codes = re.findall(r"Self::\w+ => \"([A-Z_]+)\"", src)
    codes = [c for c in codes if c.endswith("CONFLICT")]
    if not codes:
        raise SystemExit("gen_errors: ErrorCode::as_str arms not found")
    if len(set(codes)) != len(codes):
        raise SystemExit(f"gen_errors: duplicate codes: {codes}")
    return codes


def main(argv) -> int:
    args = argv or ["--check"]
    if args != ["--check"]:
        print("gen_errors: usage: `python3 tools/gen_errors.py --check`")
        return 2
    codes = parse_codes()
    spec = SPEC.read_text()
    agents = AGENTS.read_text()
    skill = SKILL.read_text()

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
    if problems:
        print(f"gen_errors: DRIFT {'; '.join(problems)}")
        print("gen_errors: cover the missing code in SPEC 25 / AGENTS.md / SKILL.md")
        return 2
    print(f"gen_errors: in sync ({len(codes)} codes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
