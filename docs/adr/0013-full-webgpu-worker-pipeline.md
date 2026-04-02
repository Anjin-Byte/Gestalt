# ADR-0013: Full WebGPU Pipeline in Renderer Worker

**Type:** adr
**Status:** superseded
**Superseded by:** [ADR-0014](0014-main-thread-gpu-worker-cpu.md)
**Supersedes:** [ADR-0011](0011-hybrid-gpu-driven.md)
**Date:** 2026-03-22
Depends on: ADR-0003 (Binary Greedy Meshing), ADR-0007 (Material Strategy), ADR-0010 (Radiance Cascades), ADR-0012 (COOP/COEP Renderer Worker)

---

## Context

ADR-0011 proposed a hybrid pipeline: custom WebGPU for chunk geometry, Three.js for debug helpers and UI overlay. This was a compromise to allow incremental migration while preserving Three.js conveniences.

Since ADR-0011, two architectural changes have made the hybrid approach unnecessary:

1. **Gestalt moved to a dedicated renderer worker** (ADR-0012). The worker owns its own `GPUDevice` via an `OffscreenCanvas` transferred from the main thread. Three.js cannot run in a worker — it requires DOM access.

2. **The legacy `apps/web/` codebase was archived**. The active application is `apps/gestalt/` with a Svelte UI. Three.js is not part of the new codebase. There is nothing to hybridize with.

The original Option C from ADR-0011 ("Replace Three.js entirely — custom WebGPU renderer for everything") is now the natural path. The concerns that argued against it are resolved:

| ADR-0011 concern about Option C | Current status |
|---|---|
| "4-6 weeks to reach feature parity" | No Three.js feature parity needed — the new app doesn't use Three.js |
| "Loses Three.js scene graph, helpers, camera controls" | Camera controls are trivial. Debug helpers become render modes in the GPU pipeline (see R-9). Scene graph is the chunk pool. |
| "Must implement material system, instanced rendering, resize handling" | Material system is specified in the vault. Instanced rendering is indirect draw. Resize is `ResizeObserver` → protocol command. |
| "No incremental migration path" | Migration is from the TS stub worker to Rust/WASM — incremental by pipeline stage |

---

## Decision

**Full WebGPU pipeline in the renderer worker. No Three.js. Rust/WASM for all GPU orchestration.**

The renderer worker (`apps/gestalt/src/renderer/renderer.worker.ts`) is a thin shell:
- Receives binary commands from the main thread via `postMessage`
- Forwards them to a Rust/WASM module that owns the `GPUDevice` and the full pipeline
- Writes frame timing to a `SharedArrayBuffer` ring buffer for the UI

All rendering logic — pipeline stages I-1 through R-9, buffer management, shader compilation, command encoding — lives in Rust compiled to WASM.

The Svelte UI layer on the main thread sends commands (camera, resize, render mode, chunk load/unload) via the binary protocol defined in ADR-0012. It reads state back via SharedArrayBuffer. It never touches WebGPU directly.

---

## Rationale

### The worker boundary enforces the separation

Three.js requires `window`, `document`, and a DOM `<canvas>`. None of these exist in a Web Worker. The moment we moved rendering to a worker, Three.js was structurally excluded. This is not a philosophical choice — it's a constraint of the execution environment.

### Rust/WASM is the right language for the pipeline

The pipeline stages (greedy meshing, DDA traversal, cascade build) involve tight loops over packed binary data (u64 bitmasks, bitfield operations, column-aware traversal). Rust compiles these to efficient WASM. The existing `greedy_mesher` crate demonstrates this — the CPU mesher is already Rust/WASM.

### Debug visualization stays GPU-side

ADR-0011 kept Three.js for debug helpers (grid, axes, bounding boxes, wireframe). These are now render modes in Stage R-9 — WGSL shaders controlled by the `SetRenderMode` command. This is faster (no JS object allocation, no Three.js scene traversal) and simpler (one pipeline, one device, one set of buffers).

### The UI is Svelte, not Three.js

All user interface — panels, outliners, property editors, performance monitors — is Svelte on the main thread (Phi component library). Three.js was never used for UI in Gestalt. The overlay pass in ADR-0011 was for debug helpers, which are now GPU render modes.

---

## Consequences

### What changes from ADR-0011

| ADR-0011 | ADR-0013 |
|---|---|
| Hybrid: custom WebGPU + Three.js overlay | Full WebGPU, no Three.js |
| Pipeline runs on main thread | Pipeline runs in renderer worker |
| TS orchestration | Rust/WASM orchestration (TS is a thin command forwarder) |
| 5-phase incremental migration from Three.js | Incremental by pipeline stage (I-1 → R-1 → R-2 → ...) |
| `apps/web/src/viewer/gpu/` | `crates/wasm_renderer/` + `apps/gestalt/src/renderer/` |
| Debug via Three.js helpers | Debug via R-9 GPU render modes |
| R-9: Three.js Overlay | R-9: Debug Visualization (render mode) |

### Implementation approach

Build the Rust/WASM renderer crate (`wasm_renderer`) incrementally by pipeline stage:

1. **Stage 0**: GPUDevice acquisition, swap chain, basic render pipeline (done — TS stub)
2. **Stage 1**: Port Stage 0 to Rust/WASM — own the device, encode commands, present
3. **Stage 2**: GPU chunk pool allocation (Rust owns buffer creation and slot management)
4. **Stage 3**: I-2 chunk occupancy upload + I-3 summary rebuild compute
5. **Stage 4**: R-1 greedy mesh compute (port binary greedy mesher to WGSL)
6. **Stage 5**: R-2 depth prepass + R-5 color pass (pixels on screen from chunk data)
7. **Stage 6**: R-3 Hi-Z + R-4 occlusion cull
8. **Stage 7**: R-6/R-7 radiance cascades
9. **Stage 8**: R-9 debug render modes

Each stage is testable independently. The TS stub in the worker is progressively replaced as each stage moves to Rust.

### What stays in TypeScript

- `RendererBridge.ts` — main thread command encoder (binary protocol)
- `protocol.ts` — shared type definitions for commands and readback buffers
- `renderer.worker.ts` — thin shell: receives messages, calls WASM entry points
- Svelte components — UI, panels, dock layout

### What moves to Rust/WASM

- GPUDevice lifecycle and pipeline creation
- Buffer allocation and chunk pool management
- Command encoding for all pipeline stages
- Camera math (projection, view matrices)
- OBJ parsing (for initial mesh loading)
- Greedy meshing (already exists in Rust, needs WGSL compute port)
- Frame timing and diagnostic counters
- All WGSL shader source strings

---

## See Also

- [ADR-0011](0011-hybrid-gpu-driven.md) — the hybrid decision this supersedes
- [ADR-0012](0012-coop-coep-renderer-worker.md) — renderer worker architecture
- [pipeline-stages](../Resident%20Representation/pipeline-stages.md) — full stage spec (R-9 updated to debug render modes)
- [demo-renderer](../Resident%20Representation/demo-renderer.md) — validation target for the initial pipeline
