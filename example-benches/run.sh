#!/usr/bin/env bash
# Rebuild every applyrs example-bench native.patch from its patch.rsmpl and
# print the token-savings table (also embedded in README.md); then measure
# every diffrs bench as staged native `git diff --cached -U0` (index lines
# stripped) vs semantic `diffrs --staged` output.
# Usage: ./example-benches/run.sh  (run from the repo root; builds the binaries if needed)
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=target/debug/applyrs
if [ ! -x "$BIN" ]; then
    cargo build -q -p applyrs
fi
DIFBIN=target/debug/diffrs
if [ ! -x "$DIFBIN" ]; then
    cargo build -q -p diffrs
fi
printf '| bench | rsmpl bytes/lines | native bytes/lines | saved |\n'
printf '|---|---|---|---|\n'
for d in example-benches/applyrs/*/; do
    name=$(basename "$d")
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
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
printf '\n| bench | native bytes/lines | semantic bytes/lines | saved |\n'
printf '|---|---|---|---|\n'
for d in example-benches/diffrs/*/; do
    name=$(basename "$d")
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    cp -r "$d/repo/." "$tmp/"
    git -C "$tmp" init -q
    git -C "$tmp" add -A
    git -C "$tmp" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm base
    cp -r "$d/after/." "$tmp/"
    # Stage the post-state so added files are covered: native comes from
    # `git diff --cached` and semantic from `diffrs --staged`. Outputs land
    # outside $tmp so measuring never feeds back into itself.
    git -C "$tmp" add -A
    git -C "$tmp" diff --cached --unified=0 --no-color --no-ext-diff | grep -v '^index ' > "$d/native.patch" || true
    "./$DIFBIN" --repo "$tmp" --staged > "$d/sem.patch" 2>/dev/null
    nb=$(wc -c < "$d/native.patch" | tr -d ' ')
    nl=$(wc -l < "$d/native.patch" | tr -d ' ')
    sb=$(wc -c < "$d/sem.patch" | tr -d ' ')
    sl=$(wc -l < "$d/sem.patch" | tr -d ' ')
    pct=$(python3 -c "print(f'{(1 - $sb / $nb) * 100:.0f}%')" )
    printf '| %s | %s B / %s ln | %s B / %s ln | %s |\n' "$name" "$nb" "$nl" "$sb" "$sl" "$pct"
    rm -rf "$tmp"
done
