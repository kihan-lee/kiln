#!/usr/bin/env bash
# dev-standard 자체의 표준 검사. --quick 은 훅 소스의 포맷과 린트만, 인자 없이 부르면 틀 검증까지 돈다.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

cargo fmt --manifest-path hook/Cargo.toml --check
cargo clippy --manifest-path hook/Cargo.toml -q -- -D warnings

if [[ "${1:-}" == "--quick" ]]; then
  exit 0
fi

bash scripts/check-template.sh
