#!/usr/bin/env bash
# Full v2 build: WASM + Vite app.
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

# Build WASM first (skip if wasm-pack not installed)
if command -v wasm-pack >/dev/null 2>&1; then
  "${SCRIPT_DIR}/wasm.sh"
else
  echo "[v2] wasm-pack not found, skipping WASM build."
fi

# Build Vite app
echo "[v2] Building app..."
pnpm -C "${ROOT_DIR}/apps/gestalt" build
echo "[v2] Build complete → apps/gestalt/dist/"
