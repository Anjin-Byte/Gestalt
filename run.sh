#!/usr/bin/env bash
set -euo pipefail

if ! command -v pnpm >/dev/null 2>&1; then
  echo "pnpm is required but not found. Install pnpm@9.12.2 and re-run."
  exit 1
fi

if [ ! -d node_modules ]; then
  echo "node_modules not found; run ./setup.sh first."
  exit 1
fi

pnpm dev
