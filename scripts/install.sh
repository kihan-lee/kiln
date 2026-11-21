#!/usr/bin/env bash
set -euo pipefail

repo="kihan-lee/kiln"
dest="${KILN_INSTALL_DIR:-$HOME/.local/bin}"
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-musl ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-musl ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  *) echo "지원하지 않는 환경: $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac
asset="kiln-$target.tar.gz"
work="$(mktemp -d)"
trap 'rm -rf "${work:?}"' EXIT

if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
  gh release download -R "$repo" ${KILN_VERSION:+"$KILN_VERSION"} -p "$asset" -D "$work"
else
  base="https://github.com/$repo/releases"
  if [[ -n "${KILN_VERSION:-}" ]]; then url="$base/download/$KILN_VERSION"; else url="$base/latest/download"; fi
  curl -fsSL "$url/$asset" -o "$work/$asset"
fi
mkdir -p "$dest"
tar -xzf "$work/$asset" -C "$work"
install -m 755 "$work/kiln" "$dest/kiln"
echo "설치됨: $dest/kiln ($("$dest/kiln" --version 2>/dev/null || echo "$target"))"
