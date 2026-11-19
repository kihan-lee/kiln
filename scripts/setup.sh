#!/usr/bin/env bash
# 장비마다 한 번 실행한다. 훅 실행 파일을 설치하고 이 저장소의 커밋 검사를 켠다.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

cargo install --path hook
command -v pre-commit >/dev/null || uv tool install pre-commit
pre-commit install
