#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-gestalt}"

if ! command -v pnpm >/dev/null 2>&1; then
  echo "pnpm is required but not found. Install pnpm@9.12.2 and re-run."
  exit 1
fi

build_with_optional_wasm() {
  local wasm_script="$1"
  local app_script="$2"

  if command -v wasm-pack >/dev/null 2>&1; then
    pnpm "${wasm_script}"
  else
    echo "wasm-pack not found; skipping WASM builds. Install wasm-pack to enable."
  fi

  pnpm "${app_script}"
}

case "${TARGET}" in
  gestalt)
    build_with_optional_wasm "build:wasm" "build"
    ;;
  legacy)
    build_with_optional_wasm "build:wasm:legacy" "build:legacy"
    ;;
  *)
    echo "Unknown target: ${TARGET}" >&2
    echo "Usage: ./build.sh [gestalt|legacy]" >&2
    exit 1
    ;;
esac
