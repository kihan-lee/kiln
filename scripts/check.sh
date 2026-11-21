#!/usr/bin/env bash
# dev-standard 자체의 검사. 훅 소스의 포맷, 린트, 테스트를 돌린다.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

cargo fmt --manifest-path hook/Cargo.toml --check
cargo clippy --manifest-path hook/Cargo.toml --all-targets -q -- -D warnings
cargo test --manifest-path hook/Cargo.toml -q
