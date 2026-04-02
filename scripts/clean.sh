#!/usr/bin/env bash
# Clean build artifacts for one or both targets.
# Usage: scripts/clean.sh [v2|legacy|all]
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${1:-all}"

clean_v2() {
  echo "[clean] Removing v2 artifacts..."
  rm -rf "${ROOT_DIR}/apps/gestalt/dist"
  rm -rf "${ROOT_DIR}/apps/gestalt/.vite"
  rm -rf "${ROOT_DIR}/apps/gestalt/src/wasm/wasm_renderer"
  rm -rf "${ROOT_DIR}/crates/wasm_renderer/target"
  echo "[clean] v2 clean."
}

clean_legacy() {
  echo "[clean] Removing legacy artifacts..."
  rm -rf "${ROOT_DIR}/legacy/apps/web/dist"
  rm -rf "${ROOT_DIR}/legacy/apps/web/.vite"
  rm -rf "${ROOT_DIR}/legacy/apps/web/src/wasm/wasm_obj_loader"
  rm -rf "${ROOT_DIR}/legacy/apps/web/src/wasm/wasm_webgpu_demo"
  rm -rf "${ROOT_DIR}/legacy/apps/web/src/wasm/wasm_voxelizer"
  rm -rf "${ROOT_DIR}/legacy/apps/web/src/wasm/wasm_greedy_mesher"
  for crate_dir in "${ROOT_DIR}"/legacy/crates/*/; do
    rm -rf "${crate_dir}target"
  done
  echo "[clean] legacy clean."
}

case "${TARGET}" in
  v2)      clean_v2 ;;
  legacy)  clean_legacy ;;
  all)     clean_v2; clean_legacy ;;
  *)
    echo "Usage: $0 [v2|legacy|all]" >&2
    exit 1
    ;;
esac
