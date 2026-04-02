#!/usr/bin/env bash
# Wrapper: delegates to scripts/v2/wasm.sh and scripts/legacy/wasm.sh.
# Kept for backward compatibility with root package.json scripts.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

case "${1:-}" in
  gestalt) "${SCRIPT_DIR}/v2/wasm.sh" ;;
  legacy)  "${SCRIPT_DIR}/legacy/wasm.sh" ;;
  all)     "${SCRIPT_DIR}/v2/wasm.sh"; "${SCRIPT_DIR}/legacy/wasm.sh" ;;
  *)       echo "Usage: $0 <gestalt|legacy|all>" >&2; exit 1 ;;
esac
