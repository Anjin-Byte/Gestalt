# 0002 - Module Contract Boundaries

**Type:** adr
**Status:** superseded
**Superseded by:** [ADR-0012](0012-coop-coep-renderer-worker.md)
**Date:** 2026-01-11

## Context
The test bed must host external WebGPU/WASM modules without coupling to module internals.

## Decision
Define a stable TypeScript interface for modules that only exchanges JSON-serializable data, typed arrays, and optional GPU device handles via a ModuleContext.

## Consequences
- The host can visualize outputs without knowing module implementation details.
- Modules remain portable and can be implemented in JS/TS or WASM.
