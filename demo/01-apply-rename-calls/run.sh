#!/usr/bin/env bash
# Demo 01: one 5-line semantic patch renames foo->bar at 12 call sites.
# Usage: ./run.sh [--keep]   (leaves the work copy behind for inspection)
set -euo pipefail
cd "$(dirname "$0")"
KEEP=0
[ "${1:-}" = "--keep" ] && KEEP=1
ROOT="$(cd ../.. && pwd)"

command -v cargo >/dev/null || { echo "error: cargo not found — install Rust from https://rustup.rs" >&2; exit 1; }
command -v git >/dev/null || { echo "error: git not found — install git first" >&2; exit 1; }
if [ ! -x "$ROOT/target/release/gitrs" ] || [ ! -x "$ROOT/target/release/applyrs" ]; then
    echo "(building gitrs + applyrs …)"
    cargo build -q --release -p gitrs -p applyrs --manifest-path "$ROOT/Cargo.toml"
fi
export PATH="$ROOT/target/release:$PATH"

work=$(mktemp -d)
if [ "$KEEP" = 1 ]; then
    echo "work copy: $work"
    trap '' EXIT
else
    trap 'rm -rf "$work"' EXIT
fi
cp -r repo/. "$work/"
cp patch.rsmpl "$work/"
git -C "$work" init -q
cd "$work"

run() { printf '$ %s\n' "$1"; eval "$1"; }
run "cat patch.rsmpl"
run "gitrs apply --file patch.rsmpl --check"
run "gitrs apply --file patch.rsmpl --output lowered.patch"
run "grep -o 'bar' src/lib.rs | wc -l"
run "wc -c patch.rsmpl lowered.patch"
