#!/usr/bin/env bash
# Build WASM for the legacy app (legacy/crates/* → legacy/apps/web/src/wasm/).
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"

CRATES=(
  "wasm_obj_loader"
  "wasm_webgpu_demo"
  "wasm_voxelizer"
  "wasm_greedy_mesher"
)

OUT_ROOT="${ROOT_DIR}/legacy/apps/web/src/wasm"
mkdir -p "${OUT_ROOT}"

for crate in "${CRATES[@]}"; do
  echo "[legacy] Building ${crate} → ${OUT_ROOT}/${crate}"
  (
    cd "${ROOT_DIR}/legacy/crates/${crate}"
    wasm-pack build --target web --out-dir "${OUT_ROOT}/${crate}"
  )
done

echo "[legacy] WASM build complete."
