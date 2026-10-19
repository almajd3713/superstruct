#!/usr/bin/env bash
# The fast loop: check and test one crate (default gen), optionally only tests matching a filter.
#   scripts/quick.sh                  gen, all tests
#   scripts/quick.sh gen l1_artery    gen, tests whose name contains l1_artery
# Tests that run past 60 s are reported and killed at 300 s (.config/nextest.toml).
set -euo pipefail
cd "$(dirname "$0")/.."
crate="${1:-gen}"
shift || true
cargo fmt --all
cargo nextest run -p "$crate" "$@"
