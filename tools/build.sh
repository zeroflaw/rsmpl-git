#!/usr/bin/env bash
# Dev-loop build: refresh gitrs + applyrs + diffrs together.
#
# `gitrs apply`/`gitrs diff` re-exec the sibling binary beside
# `current_exe`, but the `gitrs` package does not depend on either
# sibling package — so `cargo build -p gitrs` (or a single
# `--bin <one>`) correctly rebuilds only what was asked and leaves the
# re-execed sibling stale. Always build all three in one invocation.
#
# Usage: tools/build.sh [--release] [<extra cargo args>]
set -euo pipefail
exec cargo build --bin gitrs --bin applyrs --bin diffrs "$@"
