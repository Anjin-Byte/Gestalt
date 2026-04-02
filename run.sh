#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-gestalt}"

if ! command -v pnpm >/dev/null 2>&1; then
  echo "pnpm is required but not found. Install pnpm@9.12.2 and re-run."
  exit 1
fi

if [ ! -d node_modules ]; then
  echo "node_modules not found; run pnpm install first."
  exit 1
fi

case "${TARGET}" in
  gestalt)
    pnpm dev
    ;;
  legacy)
    pnpm dev:legacy
    ;;
  *)
    echo "Unknown target: ${TARGET}" >&2
    echo "Usage: ./run.sh [gestalt|legacy]" >&2
    exit 1
    ;;
esac
