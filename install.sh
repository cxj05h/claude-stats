#!/usr/bin/env bash
set -euo pipefail

REPO="cxj05h/claude-stats"
BIN_DIR="${HOME}/.local/bin"

# Detect platform
OS=$(uname -s)
ARCH=$(uname -m)

case "${OS}-${ARCH}" in
  Darwin-arm64)  TARGET="aarch64-apple-darwin" ;;
  Darwin-x86_64) TARGET="x86_64-apple-darwin" ;;
  Linux-x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
  *)
    echo "Unsupported platform: ${OS}-${ARCH}" >&2
    exit 1
    ;;
esac

VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
  | grep '"tag_name"' | sed -E 's/.*"(v[^"]+)".*/\1/')

URL="https://github.com/${REPO}/releases/download/${VERSION}/claude-stats-${TARGET}.tar.gz"

echo "Installing claude-stats ${VERSION} for ${TARGET}..."
mkdir -p "${BIN_DIR}"
curl -fsSL "${URL}" | tar xz -C "${BIN_DIR}" claude-stats
chmod +x "${BIN_DIR}/claude-stats"

echo "Installed to ${BIN_DIR}/claude-stats"
echo "Make sure ${BIN_DIR} is in your PATH."
