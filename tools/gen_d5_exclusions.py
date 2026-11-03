#!/usr/bin/env python3
"""Generate SPEC D.5 exclusion enumeration from the cycle harness table.

Source of truth: bin/diffrs/tests/cycle_e2e.rs (CYCLE_OK + KNOWN_DIVERGENT).
SPEC.md carries the block between the d5-exclusions markers; the laws stay
hand-written outside the markers, only the enumeration is generated
(IMPROVE.md WI-23b boundary).

Usage:
  python3 tools/gen_d5_exclusions.py --check   # exit 0 in sync, 2 on drift
  python3 tools/gen_d5_exclusions.py --write   # rewrite the SPEC block
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HARNESS = ROOT / "bin" / "diffrs" / "tests" / "cycle_e2e.rs"
SPEC = ROOT / "SPEC.md"

START = "<!-- generated: d5-exclusions start -- do not hand-edit; run `python3 tools/gen_d5_exclusions.py --write` -->"
END = "<!-- generated: d5-exclusions end -->"


def parse_harness(text: str):
    m = re.search(r"const CYCLE_OK[^=]*=\s*&\[(.*?)\];", text, re.S)
    if not m:
        raise SystemExit("gen_d5: CYCLE_OK block not found")
    ok = re.findall(r'"([^"]+)"', m.group(1))
    m2 = re.search(r"const KNOWN_DIVERGENT[^=]*=\s*&\[(.*?)\];", text, re.S)
    if not m2:
        raise SystemExit("gen_d5: KNOWN_DIVERGENT block not found")
    body = m2.group(1)
    # Each entry: ("name", "reason..." "cont..." ...) — join adjacent literals.
    div = []
    for em in re.finditer(
        r'\(\s*"([^"]+)"\s*,\s*((?:"(?:[^"\\]|\\.)*"\s*,?\s*)+)\)', body
    ):
        name = em.group(1)
        lits = re.findall(r'"((?:[^"\\]|\\.)*)"', em.group(2))
        reason = " ".join(lits)
        reason = re.sub(r"\s+", " ", reason).strip()
        div.append((name, reason))
    if not div:
        raise SystemExit("gen_d5: no KNOWN_DIVERGENT entries parsed")
    return ok, div


def render(ok, div) -> str:
    lines = [
        START,
        f"CYCLE_OK: {len(ok)} entries (canonically lossless; the harness asserts tree-canonical equality plus the `S'' == S'` fixed point).",
        f"KNOWN_DIVERGENT: {len(div)} entries (divergence pinned with the owning reason; a fix moves the entry to CYCLE_OK in the same change as the detector):",
    ]
    for name, reason in div:
        lines.append(f"- `{name}`: {reason}")
    lines.append(END)
    return "\n".join(lines) + "\n"


def spec_block(text: str) -> str:
    i = text.find(START)
    j = text.find(END)
    if i < 0 or j < 0 or j < i:
        raise SystemExit("gen_d5: SPEC markers not found")
    return text[i : j + len(END)] + "\n"


def main(argv) -> int:
    harness = HARNESS.read_text()
    ok, div = parse_harness(harness)
    expected = render(ok, div)
    spec = SPEC.read_text()
    current = spec_block(spec)
    if "--write" in argv:
        if current == expected:
            print(f"gen_d5: in sync ({len(ok)} ok, {len(div)} divergent)")
            return 0
        SPEC.write_text(spec.replace(current, expected))
        print(f"gen_d5: wrote ({len(ok)} ok, {len(div)} divergent)")
        return 0
    # default --check
    if current != expected:
        # Name the drift like the fence lint does (bullets only — the
        # markers themselves carry a backticked command, not entries).
        bullets = [
            m.group(1)
            for m in re.finditer(r"(?m)^- `([^`]+)`:", current)
        ]
        missing = sorted(n for n, _ in div if n not in bullets)
        stale = sorted(
            n for n in bullets if n not in {d[0] for d in div}
        )
        print(
            f"gen_d5: DRIFT CYCLE_OK={len(ok)} KNOWN_DIVERGENT={len(div)} "
            f"missing={missing} stale={stale}",
        )
        print("gen_d5: run `python3 tools/gen_d5_exclusions.py --write`")
        return 2
    print(f"gen_d5: in sync ({len(ok)} ok, {len(div)} divergent)")
    return 0


if __name__ == "__main__":
    args = sys.argv[1:] or ["--check"]
    raise SystemExit(main(args))
