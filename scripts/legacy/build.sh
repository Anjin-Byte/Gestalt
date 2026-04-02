#!/usr/bin/env bash
# Full legacy build: WASM + Vite app.
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

# Build WASM first (skip if wasm-pack not installed)
if command -v wasm-pack >/dev/null 2>&1; then
  "${SCRIPT_DIR}/wasm.sh"
else
  echo "[legacy] wasm-pack not found, skipping WASM build."
fi

# Build Vite app
echo "[legacy] Building app..."
pnpm -C "${ROOT_DIR}/legacy/apps/web" build
echo "[legacy] Build complete → legacy/apps/web/dist/"
