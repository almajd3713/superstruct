#!/usr/bin/env bash
# Format check, clippy and all tests. Run before declaring a task done.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
