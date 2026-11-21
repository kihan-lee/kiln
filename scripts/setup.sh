#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

cargo install -q --path .
command -v pre-commit >/dev/null || uv tool install pre-commit
pre-commit install
