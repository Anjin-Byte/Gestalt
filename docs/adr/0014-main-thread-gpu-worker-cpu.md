# ADR-0014: Main-Thread GPU Rendering with Worker CPU Compute

**Type:** adr
**Status:** accepted
**Supersedes:** [ADR-0013](0013-full-webgpu-worker-pipeline.md)
**Date:** 2026-03-24
Depends on: ADR-0012 (COOP/COEP Renderer Worker), ADR-0013 (Full WebGPU Worker Pipeline)

---

## Context

ADR-0013 placed the entire GPU pipeline — compute, render, and frame presentation — in a Web Worker via OffscreenCanvas. Phase 1 implementation proved the pipeline is functionally correct (74 native tests, all stages working), but revealed a frame presentation defect: **partial/incomplete frames visible during camera motion**.

### The defect

During orbit or zoom, the viewport intermittently displays frames where only a portion of the geometry is rendered. Freezing mid-motion shows sharp, fully-rendered partial frames — not blurred ghosting. The artifact affects all render modes equally (solid, wireframe, normals, depth viz), including wireframe mode which skips the R-2 depth prepass entirely. This rules out inter-pass z-fighting.

### Investigation

| Hypothesis | Test | Result |
|---|---|---|
| Z-fighting between R-2 and R-5 | Wireframe skips R-2, still tears | Ruled out |
| Present mode / frame latency | Tried Mailbox + latency=1 | wgpu panics on Mailbox (FIFO-only on web); latency=1 had no effect |
| GPU not finishing before present | Added `device.poll(Wait)` before present | No-op on WebGPU WASM backend |
| OffscreenCanvas compositor desync | Compared to legacy build (Three.js on main thread) | Legacy had no tearing |

### Root cause: wgpu `present()` is a no-op on WebGPU

From wgpu v29 source (`backend/webgpu.rs:3947-3949`):

```rust
fn present(&self) {
    // Swapchain is presented automatically on the web.
}
```

The browser compositor decides when to display the OffscreenCanvas content. In a worker, the worker's `requestAnimationFrame` is decoupled from the compositor's vsync. The compositor can grab the swapchain buffer mid-render, before all draw calls have completed on the GPU.

On the main thread, `requestAnimationFrame` is synchronized with the compositor's vsync. `getCurrentTexture()` returns a buffer that the compositor will display after the current frame's microtasks complete. This is why Three.js (main thread rendering) never has this issue.

### Industry research

| Application | Rendering thread | Worker role | Canvas type |
|---|---|---|---|
| Three.js editor | Main thread | Not used for GPU | HTMLCanvasElement |
| Godot | Main thread (native) | N/A | SubViewport (native) |
| Blender | Main thread (native) | N/A | OS region |
| Legacy Gestalt | Main thread (Three.js) | CPU meshing only | HTMLCanvasElement |

**No professional CG application renders via OffscreenCanvas in a worker.** All render on the main thread, using workers only for CPU computation.

### WebGPU threading constraints

| Object | Transferable via postMessage? | Shareable via SharedArrayBuffer? |
|---|---|---|
| GPUDevice | No (JsValue) | No |
| GPUBuffer | No (JsValue) | No |
| GPUCommandBuffer | No (JsValue) | No |
| GPUCommandEncoder | No (JsValue) | No |
| OffscreenCanvas | Yes (one-time) | No |
| ArrayBuffer | Yes (detaches from sender) | No |
| SharedArrayBuffer | Yes (shared, no transfer) | Yes |

A GPUDevice created on the main thread **cannot be used from a worker**, and vice versa. Two separate devices on the same adapter **cannot share GPU buffers** — compute results on device A are invisible to device B without CPU readback and re-upload (defeating GPU compute's purpose).

---

## Decision

**Main thread owns all GPU work. Worker does CPU-only Rust/WASM computation.**

### Main thread responsibilities

- Own GPUDevice and GPUQueue
- Create and manage all GPU buffers (chunk pool)
- Upload data to GPU (`queue.writeBuffer`)
- Dispatch all compute passes: I-3 (summary rebuild), R-1 (greedy mesh), build_indirect, build_wireframe
- Execute all render passes: R-2 (depth prepass), R-5 (color), R-9 (debug viz)
- Drive render loop via `requestAnimationFrame` (compositor-synced)
- Handle canvas resize via ResizeObserver → surface reconfigure
- Load WASM module (`wasm_renderer`) directly — no worker intermediary for GPU calls

### Worker responsibilities

- Heavy CPU computation in Rust/WASM:
  - OBJ parsing and spatial indexing (Phase 2)
  - CPU-side voxelization fallback
  - Chunk data preparation for streaming (Phase 6)
  - Validation and test oracle computation
- Post results as transferable ArrayBuffers → main thread uploads to GPU
- SharedArrayBuffer for bidirectional stats and diagnostics

### Canvas model

- Regular `HTMLCanvasElement` — no `transferControlToOffscreen`, no OffscreenCanvas
- Phi provides a `Viewport` panel primitive that owns the canvas element
- Canvas managed outside Svelte's reactive framework (raw DOM, like Three.js editor)

### Communication

- Worker → main: transferable `ArrayBuffer` (mesh data, occupancy arrays)
- Main → worker: command `ArrayBuffer` (load requests, edit commands)
- Bidirectional: `SharedArrayBuffer` for frame timing and diagnostics (existing infrastructure)

---

## Rationale

### Compositor synchronization is non-negotiable

The WebGPU specification defines frame presentation timing in terms of the document's rendering lifecycle. `getCurrentTexture()` on a canvas context returns a texture that the compositor displays at the next vsync **after the current task completes**. In a worker, "the current task" is the worker's rAF callback — which is not synchronized with the compositor's vsync. There is no API to force synchronization from a worker.

This is not a wgpu bug or a browser bug. It is the specified behavior of OffscreenCanvas rendering in workers. The compositor is allowed to display the most recent completed frame at any time, and a frame is "completed" when `getCurrentTexture()` is called for the next frame — not when `queue.submit()` finishes.

### GPU compute dispatches are cheap on the main thread

The concern that drove ADR-0013 was keeping heavy GPU work off the main thread to avoid UI jank. This concern is misplaced for GPU compute:

- `device.createCommandEncoder()` + `pass.dispatch()` + `queue.submit()` are CPU-side calls that take microseconds
- The actual GPU work (I-3 bricklet scan, R-1 greedy merge, R-2/R-5 draw) executes asynchronously on the GPU
- The main thread returns immediately after `submit()` and handles UI events while the GPU works
- Only truly heavy **CPU** computation (OBJ parsing, spatial indexing) should be in a worker

### The legacy architecture worked

The legacy Gestalt build used this exact pattern: Three.js rendered on the main thread, the worker did CPU meshing and posted vertex buffers back. No frame tearing, no partial frames. ADR-0014 returns to this proven pattern with Rust/WASM replacing Three.js.

---

## Consequences

### What changes from ADR-0013

| ADR-0013 | ADR-0014 |
|---|---|
| Full pipeline in worker | GPU on main thread, CPU in worker |
| Worker owns GPUDevice | Main thread owns GPUDevice |
| OffscreenCanvas transferred to worker | HTMLCanvasElement stays on main thread |
| `renderer.worker.ts` drives render loop | Main thread rAF drives render loop |
| WASM loaded in worker | WASM loaded on main thread (GPU module) + worker (CPU module) |
| Binary command protocol (main → worker → GPU) | Direct WASM calls on main thread for GPU; protocol for worker CPU tasks |

### Migration path

1. Move WASM GPU module loading from worker to main thread
2. Replace OffscreenCanvas transfer with direct canvas access
3. Move `requestAnimationFrame` loop from worker to main thread
4. Remove binary command protocol for GPU operations (direct WASM calls instead)
5. Keep worker for future CPU-heavy tasks (OBJ parsing, streaming)
6. Keep SharedArrayBuffer for stats/diagnostics

### Phi viewport primitive

Phi needs a `Viewport` component that:
- Creates and owns a raw `<canvas>` element within a dock panel
- Exposes the canvas for direct GPU context acquisition
- Handles resize via ResizeObserver
- Routes pointer events for camera control
- Is not managed by Svelte's reactive DOM (raw element, like Three.js `renderer.domElement`)

### What stays unchanged

- All Rust/WASM GPU code (`wasm_renderer` crate) — same shaders, same passes, same pool
- Phi panel system and UI components
- SharedArrayBuffer diagnostics infrastructure
- 74 native tests — all platform-independent, unaffected by thread boundary

---

## See Also

- [ADR-0013](0013-full-webgpu-worker-pipeline.md) — the full-worker decision this supersedes
- [ADR-0012](0012-coop-coep-renderer-worker.md) — COOP/COEP and worker architecture (still relevant for CPU worker)
- [ADR-0011](0011-hybrid-gpu-driven.md) — the earlier hybrid Three.js decision (historical)
- [thread-boundary](../Resident%20Representation/thread-boundary.md) — exact split of work between threads
- [viewport-architecture](../Resident%20Representation/viewport-architecture.md) — Phi viewport component model
- [roadmap](../architecture/roadmap.md) — Phase 1.5 cleanup entry
