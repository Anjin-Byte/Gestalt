# Thread Boundary: Main Thread GPU / Worker CPU

**Type:** spec
**Status:** current
**Date:** 2026-03-24
**Depends on:** ADR-0014

> Defines exactly which work runs on which thread, what crosses the boundary, and how synchronization works.

---

## Principle

GPU objects (device, buffers, encoders, pipelines) cannot cross Web Worker boundaries. The main thread owns all GPU state. The worker does CPU-only computation and posts results as transferable ArrayBuffers.

---

## Main Thread (GPU + UI)

### Owns

| Resource | Notes |
|---|---|
| `GPUDevice` | Single device from `navigator.gpu.requestAdapter().requestDevice()` |
| `GPUQueue` | Single queue, all submissions ordered |
| All GPU buffers | Chunk pool (occupancy, palette, coord, vertex, index, draw_meta, indirect, etc.) |
| All compute pipelines | I-3 summary, R-1 mesh, build_indirect, build_wireframe |
| All render pipelines | R-2 depth, R-5 color, R-9 debug modes (normals, wireframe, depth viz) |
| Surface / swapchain | `HTMLCanvasElement.getContext("webgpu")`, configured per frame |
| Camera uniform buffer | Updated every frame before render |
| WASM GPU module | `wasm_renderer` crate loaded via `wasm-bindgen` on main thread |

### Executes

| Per-frame work | Type | Cost |
|---|---|---|
| Camera uniform upload | `queue.writeBuffer` | ~μs |
| I-3 summary dispatch (if dirty) | Compute | ~μs encode, GPU async |
| R-1 mesh dispatch (if dirty) | Compute | ~μs encode, GPU async |
| build_indirect dispatch | Compute | ~μs encode, GPU async |
| build_wireframe dispatch | Compute | ~μs encode, GPU async |
| R-2 depth prepass | Render | ~μs encode, GPU async |
| R-5 color pass | Render | ~μs encode, GPU async |
| `queue.submit()` | Submission | ~μs, GPU work is async |
| Frame presentation | Implicit | Compositor-synced via rAF |

All GPU dispatches are **asynchronous** — `queue.submit()` returns immediately. The GPU executes in parallel while the main thread handles UI events. Main thread CPU cost per frame is command encoding only (microseconds).

### Drives

- `requestAnimationFrame` render loop (compositor-synced)
- ResizeObserver → surface reconfigure
- Pointer events → camera orbit → camera uniform update
- Svelte UI / Phi panels

---

## Worker Thread (CPU Only)

### Owns

| Resource | Notes |
|---|---|
| WASM CPU module | Loaded separately — only platform-independent Rust code (no wgpu, no web_sys) |
| CPU computation results | Occupancy arrays, mesh data, validation results |

### Executes (future phases)

| Task | Phase | Notes |
|---|---|---|
| OBJ parsing | Phase 2 | Parse mesh file → vertex/index/material arrays |
| Spatial indexing | Phase 2 | Triangle → chunk assignment for voxelization |
| CPU voxelization fallback | Phase 2 | SAT-based triangle-box overlap |
| Chunk data preparation | Phase 6 | Streaming: prepare occupancy for upload |
| Test oracle computation | Debug | CPU reference mesher/summary for validation |

### Does NOT touch

- GPUDevice, GPUBuffer, GPUCommandEncoder, GPUCommandBuffer
- Canvas, Surface, Swapchain
- DOM, Window, Document

---

## Boundary Crossings

### Worker → Main Thread

| Data | Transport | Format | When |
|---|---|---|---|
| Mesh data (verts + indices) | Transferable ArrayBuffer | Raw bytes, zero-copy | After CPU mesh/parse complete |
| Occupancy arrays | Transferable ArrayBuffer | `[u32; 8192]` per chunk | After voxelization |
| Chunk metadata | postMessage (small) | JSON-like structured clone | With mesh data |
| Stats / diagnostics | SharedArrayBuffer | Fixed-layout, lock-free reads | Continuous |

### Main Thread → Worker

| Data | Transport | Format | When |
|---|---|---|---|
| Load requests | postMessage | Command buffer (future) | On user action |
| Edit commands | postMessage | Command buffer (future: Phase 4) | On user action |

### Not Crossed

| Data | Why |
|---|---|
| GPU buffers | JsValue, not transferable |
| GPU pipelines | JsValue, not transferable |
| GPU device/queue | JsValue, not transferable |
| Svelte stores | Main-thread-only reactive state |

---

## Synchronization

### GPU work ordering

All GPU submissions go through a single `GPUQueue` on the main thread. WebGPU guarantees in-order execution within a queue. No explicit fences or barriers needed between compute and render passes submitted in the same `queue.submit()` call.

### Worker → main thread data handoff

1. Worker completes CPU computation
2. Worker posts ArrayBuffer via `postMessage` with transfer list (zero-copy, detaches from worker)
3. Main thread receives ArrayBuffer in message handler
4. Main thread uploads to GPU via `queue.writeBuffer()`
5. Main thread dispatches compute/render passes that read the data
6. `queue.submit()` guarantees writeBuffer completes before dispatch reads

### Frame timing

Main thread's `requestAnimationFrame` is the heartbeat:
1. rAF callback fires (compositor-synced)
2. Check for pending worker data (posted ArrayBuffers in message queue)
3. Upload any new data to GPU
4. Dispatch compute if dirty
5. Dispatch render passes
6. `queue.submit()` — GPU works async
7. Compositor presents the frame at next vsync

---

## WASM Module Split

### Main thread module: `wasm_renderer`

Contains all GPU-dependent code:
- `lib.rs` — Renderer struct, render_frame(), load_test_scene()
- `pool_gpu.rs` — ChunkPool (buffer creation, bind groups, upload)
- `gpu.rs` — RenderResources (pipelines, camera uniform)
- `passes/` — SummaryPass, MeshPass, BuildIndirectPass, BuildWireframePass
- `shaders/` — All WGSL shader source
- `camera.rs` — Camera math (platform-independent but used on main thread)

### Worker module: `wasm_compute` (future, Phase 2+)

Contains CPU-only Rust code:
- OBJ parser
- Spatial indexing (CSR triangle lists)
- CPU voxelization (SAT overlap)
- No wgpu dependency, no web_sys GPU features
- Compiles to `wasm32-unknown-unknown` without browser-specific features

### Shared code (both modules)

- `pool.rs` — Constants, DrawMeta, SlotAllocator (platform-independent)
- `scene.rs` — OccupancyBuilder, PaletteBuilder, MaterialEntry
- `summary_cpu.rs` — CPU reference I-3
- `mesh_cpu.rs` — CPU reference R-1

---

## See Also

- [ADR-0014](../adr/0014-main-thread-gpu-worker-cpu.md) — the decision establishing this boundary
- [viewport-architecture](viewport-architecture.md) — Phi viewport component model
- [pipeline-stages](pipeline-stages.md) — stage execution order and buffer ownership
