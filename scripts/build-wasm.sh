#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

CRATES=(
  "wasm_obj_loader"
  "wasm_webgpu_demo"
  "wasm_voxelizer"
  "wasm_greedy_mesher"
)

# The main renderer crate — built separately for the gestalt app.
RENDERER_CRATE="wasm_renderer"

usage() {
  echo "Usage: $0 <gestalt|legacy|all>" >&2
  exit 1
}

build_target() {
  local target_name="$1"
  local out_root="$2"

  mkdir -p "${ROOT_DIR}/${out_root}"

  for crate in "${CRATES[@]}"; do
    echo "Building ${crate} -> ${out_root}/${crate}"
    (
      cd "${ROOT_DIR}/crates/${crate}"
      wasm-pack build \
        --target web \
        --out-dir "${ROOT_DIR}/${out_root}/${crate}"
    )
  done

  echo "Finished WASM build for ${target_name}."
}

build_renderer() {
  echo "Building ${RENDERER_CRATE} -> apps/gestalt/src/wasm/${RENDERER_CRATE}"
  (
    cd "${ROOT_DIR}/crates/${RENDERER_CRATE}"
    wasm-pack build \
      --target web \
      --out-dir "${ROOT_DIR}/apps/gestalt/src/wasm/${RENDERER_CRATE}"
  )
}

case "${1:-}" in
  gestalt)
    build_renderer
    build_target "gestalt" "apps/gestalt/src/wasm"
    ;;
  legacy)
    build_target "legacy" "legacy/apps/web/src/wasm"
    ;;
  all)
    build_renderer
    build_target "gestalt" "apps/gestalt/src/wasm"
    build_target "legacy" "legacy/apps/web/src/wasm"
    ;;
  *)
    usage
    ;;
esac
