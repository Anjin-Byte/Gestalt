# 0002 - Module Contract Boundaries

## Status
Accepted

## Context
The test bed must host external WebGPU/WASM modules without coupling to module internals.

## Decision
Define a stable TypeScript interface for modules that only exchanges JSON-serializable data, typed arrays, and optional GPU device handles via a ModuleContext.

## Consequences
- The host can visualize outputs without knowing module implementation details.
- Modules remain portable and can be implemented in JS/TS or WASM.
