#!/usr/bin/env bash
# Build WASM for the v2 renderer (crates/wasm_renderer → apps/gestalt/src/wasm/).
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"

CRATE="wasm_renderer"
OUT="${ROOT_DIR}/apps/gestalt/src/wasm/${CRATE}"

echo "[v2] Building ${CRATE} → ${OUT}"
(
  cd "${ROOT_DIR}/crates/${CRATE}"
  wasm-pack build --target web --out-dir "${OUT}"
)
echo "[v2] WASM build complete."
