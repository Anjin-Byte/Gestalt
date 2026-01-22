#!/usr/bin/env bash
set -euo pipefail

if ! command -v pnpm >/dev/null 2>&1; then
  echo "pnpm is required but not found. Install pnpm@9.12.2 and re-run."
  exit 1
fi

if command -v wasm-pack >/dev/null 2>&1; then
  pnpm build:wasm
else
  echo "wasm-pack not found; skipping WASM builds. Install wasm-pack to enable."
fi

pnpm build
