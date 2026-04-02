#!/usr/bin/env bash
# Start the legacy dev server with hot reload.
# WASM must be built first — run scripts/legacy/wasm.sh if needed.
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"

echo "[legacy] Starting dev server..."
pnpm -C "${ROOT_DIR}/legacy/apps/web" dev
