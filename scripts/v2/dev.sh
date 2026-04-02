#!/usr/bin/env bash
# Start the v2 dev server with hot reload.
# WASM must be built first — run scripts/v2/wasm.sh if needed.
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"

echo "[v2] Starting dev server..."
pnpm -C "${ROOT_DIR}/apps/gestalt" dev
