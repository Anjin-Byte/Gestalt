#!/usr/bin/env bash
# Run all legacy tests: Rust crate tests + wasm-bindgen browser tests.
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"

FAILED=0

# Rust unit tests (greedy_mesher + voxelizer)
echo "[legacy] Running greedy_mesher tests..."
if (cd "${ROOT_DIR}/legacy/crates/greedy_mesher" && cargo test); then
  echo "[legacy] greedy_mesher: PASS"
else
  echo "[legacy] greedy_mesher: FAIL"
  FAILED=1
fi

echo ""
echo "[legacy] Running voxelizer tests..."
if (cd "${ROOT_DIR}/legacy/crates/voxelizer" && cargo test); then
  echo "[legacy] voxelizer: PASS"
else
  echo "[legacy] voxelizer: FAIL"
  FAILED=1
fi

# wasm-bindgen browser tests (requires wasm-pack + Chrome)
if command -v wasm-pack >/dev/null 2>&1; then
  echo ""
  echo "[legacy] Running wasm-bindgen tests (wasm_greedy_mesher)..."
  if (cd "${ROOT_DIR}/legacy/crates/wasm_greedy_mesher" && wasm-pack test --headless --chrome); then
    echo "[legacy] wasm-bindgen tests: PASS"
  else
    echo "[legacy] wasm-bindgen tests: FAIL"
    FAILED=1
  fi
else
  echo ""
  echo "[legacy] wasm-pack not found, skipping wasm-bindgen tests."
fi

echo ""
if [ "$FAILED" -eq 0 ]; then
  echo "[legacy] All tests passed."
else
  echo "[legacy] Some tests failed."
  exit 1
fi
