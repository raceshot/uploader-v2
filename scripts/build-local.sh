#!/usr/bin/env bash
# Build for the current platform only.
# Usage:
#   ./scripts/build-local.sh              # current arch
#   ./scripts/build-local.sh --target aarch64-apple-darwin

set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> Installing frontend dependencies..."
pnpm install

echo "==> Building Tauri bundle ($*)..."
pnpm tauri build "$@"

echo ""
echo "==> Bundle output:"
find src-tauri/target/release/bundle -maxdepth 3 \( -name "*.dmg" -o -name "*.msi" -o -name "*.AppImage" -o -name "*.deb" \) 2>/dev/null | sort
