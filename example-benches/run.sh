#!/usr/bin/env bash
# Rebuild every example-bench native.patch from its patch.rsmpl and print
# the token-savings table (also embedded in README.md).
# Usage: ./run.sh  (run from the repo root; builds the binary if needed)
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=target/debug/rsmpl-git
if [ ! -x "$BIN" ]; then
    cargo build -q -p rsmpl-git
fi
printf '| bench | rsmpl bytes/lines | native bytes/lines | saved |\n'
printf '|---|---|---|---|\n'
for d in example-benches/*/; do
    name=$(basename "$d")
    tmp=$(mktemp -d)
    cp -r "$d/repo/." "$tmp/"
    git -C "$tmp" init -q
    "./$BIN" --file "$d/patch.rsmpl" --repo "$tmp" --output "$d/native.patch" >/dev/null
    rb=$(wc -c < "$d/patch.rsmpl" | tr -d ' ')
    rl=$(wc -l < "$d/patch.rsmpl" | tr -d ' ')
    nb=$(wc -c < "$d/native.patch" | tr -d ' ')
    nl=$(wc -l < "$d/native.patch" | tr -d ' ')
    pct=$(python3 -c "print(f'{(1 - $rb / $nb) * 100:.0f}%')" )
    printf '| %s | %s B / %s ln | %s B / %s ln | %s |\n' "$name" "$rb" "$rl" "$nb" "$nl" "$pct"
    rm -rf "$tmp"
done
