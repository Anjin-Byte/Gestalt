#!/usr/bin/env bash
# Run all v2 tests: Rust unit tests, Phi component tests, Playwright e2e.
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"

FAILED=0

# Rust unit tests (wasm_renderer crate — platform-independent code)
echo "[v2] Running Rust tests..."
if (cd "${ROOT_DIR}/crates/wasm_renderer" && cargo test); then
  echo "[v2] Rust tests: PASS"
else
  echo "[v2] Rust tests: FAIL"
  FAILED=1
fi

# Phi component tests
echo ""
echo "[v2] Running Phi tests..."
if pnpm -C "${ROOT_DIR}/packages/phi" test; then
  echo "[v2] Phi tests: PASS"
else
  echo "[v2] Phi tests: FAIL"
  FAILED=1
fi

# Playwright e2e (optional — only if browsers are installed)
if command -v playwright >/dev/null 2>&1 || [ -d "${ROOT_DIR}/apps/gestalt/node_modules/.cache/ms-playwright" ]; then
  echo ""
  echo "[v2] Running Playwright tests..."
  if pnpm -C "${ROOT_DIR}/apps/gestalt" test:e2e; then
    echo "[v2] Playwright tests: PASS"
  else
    echo "[v2] Playwright tests: FAIL"
    FAILED=1
  fi
else
  echo ""
  echo "[v2] Playwright not installed, skipping e2e tests."
fi

echo ""
if [ "$FAILED" -eq 0 ]; then
  echo "[v2] All tests passed."
else
  echo "[v2] Some tests failed."
  exit 1
fi
