#!/usr/bin/env bash
# dev-standard 자체의 검사. 훅 소스의 포맷과 린트, 템플릿 검증을 돌린다. 커밋한 뒤 실행한다.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

cargo fmt --manifest-path hook/Cargo.toml --check
cargo clippy --manifest-path hook/Cargo.toml -q -- -D warnings
cargo test --manifest-path hook/Cargo.toml -q
bash scripts/check-template.sh
