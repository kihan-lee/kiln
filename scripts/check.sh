#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

cargo fmt --check
cargo clippy --all-targets -q -- -D warnings
cargo test -q
