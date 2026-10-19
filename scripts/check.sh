#!/usr/bin/env bash
# The full check: format, clippy and every test. Run at critical points (before a commit that
# closes an item, phase exits). While working, use scripts/quick.sh on the crate you touch.
set -euo pipefail
cd "$(dirname "$0")/.."

# Every idea of the Web redesign must have exactly one owning checklist item.
python3 scripts/coverage.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
# Optional features are off in the workspace build above; check them explicitly.
cargo clippy -p gen --all-targets --features serde -- -D warnings
cargo nextest run -p gen --features serde
