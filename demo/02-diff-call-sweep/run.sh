#!/usr/bin/env bash
# Demo 02: gitrs diff compresses 12 changed call sites into one 5-line
# semantic patch — the same change `git diff` spells out line by line.
# Usage: ./run.sh [--keep]   (leaves the work copy behind for inspection)
set -euo pipefail
cd "$(dirname "$0")"
KEEP=0
[ "${1:-}" = "--keep" ] && KEEP=1
ROOT="$(cd ../.. && pwd)"
HERE="$(pwd)"

command -v cargo >/dev/null || { echo "error: cargo not found — install Rust from https://rustup.rs" >&2; exit 1; }
command -v git >/dev/null || { echo "error: git not found — install git first" >&2; exit 1; }
if [ ! -x "$ROOT/target/release/gitrs" ] || [ ! -x "$ROOT/target/release/diffrs" ]; then
    echo "(building gitrs + diffrs …)"
    cargo build -q --release -p gitrs -p diffrs --manifest-path "$ROOT/Cargo.toml"
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
git -C "$work" init -q
# Ignore patch outputs: diffrs reads untracked files too, so committed
# measurement artifacts must stay invisible to later diffs.
printf '*.patch\n' > "$work/.gitignore"
git -C "$work" add -A
git -C "$work" -c commit.gpgsign=false -c user.email=demo@local -c user.name=demo commit -qm base
cp -r after/. "$work/"
cd "$work"

run() { printf '$ %s\n' "$1"; eval "$1"; }
run "gitrs diff --repo ."
# diffrs first: it reads untracked files too, so capture the semantic
# patch before native.patch exists. Plain `git diff` ignores untracked
# files, so it is unaffected by sem.patch.
run "gitrs diff --repo . > sem.patch"
run "git diff --unified=0 --no-color | grep -v '^index ' > native.patch"
run "wc -c native.patch sem.patch"
