#!/usr/bin/env python3
"""Generate the SPEC D.4 guard table from the `guard_for` mapping.

Source of truth: the code itself —
  crates/rsmpl_diff_infer/src/semantic/guard.rs (`guard_for` Precision arms)
  crates/rsmpl_diff_infer/src/semantic/definition_evolution.rs (`PRECISION_GUARD`)
SPEC.md carries the blocks between the guard markers; the guard laws stay
hand-written outside the markers, only the membership table is generated
(IMPROVE.md WI-9 boundary).

Usage:
  python3 tools/gen_guard.py --check   # exit 0 in sync, 2 on drift
  python3 tools/gen_guard.py --write   # rewrite the SPEC blocks
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GUARD = ROOT / "crates" / "rsmpl_diff_infer" / "src" / "semantic" / "guard.rs"
EVOLUTION = (
    ROOT / "crates" / "rsmpl_diff_infer" / "src" / "semantic" / "definition_evolution.rs"
)
SPEC = ROOT / "SPEC.md"

START = "<!-- generated: guard start -- do not hand-edit; run `python3 tools/gen_guard.py --write` -->"
END = "<!-- generated: guard end -->"
CSTART = "<!-- generated: contract-guard start -- do not hand-edit; run `python3 tools/gen_guard.py --write` -->"
CEND = "<!-- generated: contract-guard end -->"


def parse_guard():
    src = GUARD.read_text()
    m = re.search(r"fn guard_for\(.*?match detector \{(.*?)\n\s*_\s*=>", src, re.S)
    if not m:
        raise SystemExit("gen_guard: guard_for Precision arms not found")
    arm = m.group(1)
    precision = re.findall(r'"([^"]+)"', arm)
    if not precision:
        raise SystemExit("gen_guard: no Precision detectors parsed (partial table refused)")
    if len(set(precision)) != len(precision):
        raise SystemExit(f"gen_guard: duplicate Precision detectors: {precision}")
    return precision


def parse_const():
    src = EVOLUTION.read_text()
    m = re.search(r"const PRECISION_GUARD:\s*usize\s*=\s*(\d+)", src)
    if not m:
        raise SystemExit("gen_guard: PRECISION_GUARD const not found")
    return int(m.group(1))


def render(precision, const) -> str:
    rows = "\n".join(f"| `{d}` | Precision |" for d in precision)
    return (
        f"{START}\n"
        f"Guard posture (code is the source: `guard_for` in `guard.rs`, "
        f"`PRECISION_GUARD` = {const}): {len(precision)} Precision detectors; "
        f"every other detector rides Strict.\n"
        f"| Detector | Guard |\n"
        f"|---|---|\n"
        f"{rows}\n"
        f"{END}\n"
    )


def render_contract(precision, const) -> str:
    return (
        f"{CSTART}\n"
        f"Guards: {len(precision)} Precision, rest Strict "
        f"(`PRECISION_GUARD` = {const}; SPEC D.4, generated block).\n"
        f"{CEND}\n"
    )


def block(text: str, start: str, end: str) -> str:
    i = text.find(start)
    j = text.find(end)
    if i < 0 or j < 0 or j < i:
        raise SystemExit(f"gen_guard: markers not found: {start[:32]}...")
    return text[i : j + len(end)] + "\n"


def main(argv) -> int:
    precision = parse_guard()
    const = parse_const()
    expected = render(precision, const)
    expected_c = render_contract(precision, const)
    spec = SPEC.read_text()
    current = block(spec, START, END)
    current_c = block(spec, CSTART, CEND)
    if "--write" in argv:
        changed = False
        if current != expected:
            spec = spec.replace(current, expected)
            changed = True
        if current_c != expected_c:
            spec = spec.replace(current_c, expected_c)
            changed = True
        if changed:
            SPEC.write_text(spec)
            print(f"gen_guard: wrote ({len(precision)} precision, const={const})")
        else:
            print(f"gen_guard: in sync ({len(precision)} precision, const={const})")
        return 0
    # default --check
    problems = []
    if current != expected:
        problems.append("SPEC-D.4-guard")
    if current_c != expected_c:
        problems.append("SPEC-head-contract-guard")
    if problems:
        print(f"gen_guard: DRIFT {problems} ({len(precision)} precision, const={const})")
        print("gen_guard: run `python3 tools/gen_guard.py --write`")
        return 2
    print(f"gen_guard: in sync ({len(precision)} precision, const={const})")
    return 0


if __name__ == "__main__":
    args = sys.argv[1:] or ["--check"]
    raise SystemExit(main(args))
