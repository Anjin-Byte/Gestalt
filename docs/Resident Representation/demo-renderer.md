# Demo Renderer

**Type:** spec
**Status:** current
**Date:** 2026-03-22

Spec for the GPU-resident demo pipeline. Runs in the renderer worker via Rust/WASM + WebGPU, renders to an OffscreenCanvas.

The purpose of this demo is validation: prove the resident architecture described in this vault works end-to-end before building the full pipeline.

---

## Scope

The demo renderer is the renderer worker itself (`apps/gestalt/src/renderer/renderer.worker.ts`) calling into a Rust/WASM crate (`wasm_renderer`). It is not a separate module or standalone canvas — it IS the application's renderer.

**In scope for the initial demo:**
- Rust/WASM owns GPUDevice via OffscreenCanvas (transferred from main thread)
- Hardcoded procedural test scene (no file I/O, no OBJ loading)
- GPU chunk pool (layouts from [gpu-chunk-pool](gpu-chunk-pool.md))
- Depth prepass + Hi-Z build + occlusion cull + color pass (Stages R-2 through R-5)
- Orbit camera controlled via binary protocol commands from main thread
- Cascade 0 only — deferred full cascade merge until [radiance-cascades-impl](radiance-cascades-impl.md) is complete

**Out of scope for the initial demo:**
- Radiance cascade merge or multi-cascade GI (placeholder ambient only)
- Edit protocol / dirty tracking / runtime voxel edits
- Streaming or LRU eviction (fixed scene, all chunks resident)
- LOD / far-field approximations

---

## Architecture

```
Main Thread (Svelte)                    Worker Thread (Rust/WASM)
┌─────────────────────┐                 ┌──────────────────────────────┐
│ RendererBridge.ts   │ ──commands──>   │ renderer.worker.ts (thin TS) │
│ (binary protocol)   │                 │   └── wasm_renderer (Rust)   │
│                     │ <──SAB ring──   │       ├── GPUDevice          │
│ Svelte Panels       │                 │       ├── chunk pool         │
│ (read SAB for       │                 │       ├── pipeline stages    │
│  diagnostics)       │                 │       └── WGSL shaders       │
└─────────────────────┘                 └──────────────────────────────┘
```

The TS worker file is a thin shell:
1. Allocates SharedArrayBuffers for readback
2. Receives `init-canvas` message with transferred OffscreenCanvas
3. Loads and initializes the Rust/WASM module, passing it the GPU device
4. Forwards all subsequent binary commands to WASM
5. Calls `requestAnimationFrame` to drive the WASM frame loop

All GPU work — buffer creation, command encoding, compute dispatch, render passes — is in Rust.

---

## Crate Location

```
crates/wasm_renderer/
  Cargo.toml
  src/
    lib.rs              — WASM entry point, GPUDevice ownership
    pool.rs             — GPU chunk pool (slot allocation, buffer init)
    scene.rs            — procedural test scene (hardcoded chunks)
    camera.rs           — projection, view matrices, orbit controls
    passes/
      mod.rs
      summary_rebuild.rs  — I-3: compute summary, flags, aabb
      mesh_rebuild.rs     — R-1: greedy mesh compute dispatch
      depth_prepass.rs    — R-2: depth-only render pass
      hiz_build.rs        — R-3: Hi-Z pyramid compute
      occlusion_cull.rs   — R-4: chunk AABB cull compute
      color_pass.rs       — R-5: main color render pass
    shaders/
      depth.wgsl
      color.wgsl
      hiz_build.wgsl
      occlusion_cull.wgsl
      mesh_rebuild.wgsl
      summary_rebuild.wgsl
```

---

## Test Scene

Hardcoded procedural scene. No file I/O at startup.

### Room

A hollow axis-aligned room, 8 × 8 × 8 chunks (512³ voxels). Floor, ceiling, and four walls filled solid. Interior empty except for a central object and emissive voxels.

### Central Object

A solid sphere of radius ~20 voxels at the center of the room. Dense enough to produce interesting Hi-Z culling behavior on the chunks behind it from the default camera.

### Emissive Voxels

Four clusters of emissive voxels at the corners of the room interior. These are the light sources for cascade 0 when radiance cascades are eventually integrated. For the MVP, they are rendered with a bright emissive tint in the color pass but do not yet contribute to GI.

### Chunk Population

All chunks with any occupied voxels are resident from the start. Empty interior chunks are marked `is_empty` in `chunk_flags` and skipped by the traversal and cull passes.

Expected resident chunk count: ~200–300 for this scene (room shell + central sphere + emissive clusters). Well within the 1024-slot pool.

---

## GPU Resource Initialization

On `init-canvas` message, before the first frame:

```
1. Acquire GPUAdapter + GPUDevice from the OffscreenCanvas
2. Allocate chunk pool buffers (see gpu-chunk-pool.md)
3. Populate chunk_occupancy_atlas for all resident chunks (writeBuffer)
4. Populate chunk_coord, chunk_slot_table_gpu (writeBuffer)
5. Dispatch summary_rebuild for all resident chunks (compute)
6. Dispatch mesh_rebuild for all resident chunks (compute)
7. Wait for completion (onSubmittedWorkDone) before rendering first frame
```

No incremental rebuild on first frame — full blocking init pass.

---

## Frame Loop

```
Each frame (driven by requestAnimationFrame in the worker):

1. Read pending commands from main thread (camera, resize, render mode)
2. Update camera uniforms (view, proj, camera_pos, camera_forward)

3. CPU sort: order chunk draw_metadata by camera distance
   Write sorted indirect draw args to staging buffer

4. Stage R-2: Depth prepass
   Render all resident (non-empty) chunks to depth_texture, depth-only

5. Stage R-3: Hi-Z build
   Dispatch hiz_build.wgsl: downsample depth_texture → hiz_pyramid

6. Stage R-4: Occlusion cull
   Dispatch occlusion_cull.wgsl: test chunk AABBs vs. hiz_pyramid
   Write surviving draw calls to indirect_draw_buf

7. Stage R-5: Color pass
   Draw surviving chunks via indirect_draw_buf
   Fragment: material lookup + placeholder ambient

8. Stage R-9: Debug visualization (if render mode != Default)
   Composite debug overlay based on SetRenderMode command

9. Write frame timing to SharedArrayBuffer ring buffer
10. Present
```

No edit protocol, no dirty propagation, no rebuild work after init.

---

## Camera

Orbit camera. Main thread sends `SetCamera` commands via binary protocol.

Default position: outside and above the room, looking inward at the central sphere.

The Rust camera module computes projection and view matrices from the received position + direction. Full perspective projection with configurable FOV, near, far.

---

## Binary Protocol Integration

The demo renderer receives all input via the binary protocol (see `protocol.ts`):

| Command | Effect in renderer |
|---|---|
| `SetCamera` | Update view matrix |
| `ResizeViewport` | Recreate swap chain + depth texture |
| `SetRenderMode` | Switch R-9 debug visualization |
| `LoadChunk` | Allocate pool slot, upload occupancy, rebuild summary + mesh |
| `UnloadChunk` | Free pool slot |

Frame timing is written back via the SharedArrayBuffer ring buffer. The Svelte PerformancePanel reads it on the main thread without any message passing.

---

## Validation Criteria

The demo is complete when:

1. Scene renders correctly at 60fps on a desktop WebGPU-capable browser
2. Depth prepass populates `depth_texture` visibly correctly (depth debug mode)
3. Hi-Z pyramid visibly correct at each mip level (Hi-Z debug mode)
4. Occlusion cull removes expected chunks (chunks behind the central sphere culled)
5. Draw call count drops measurably when camera faces the sphere vs. away
6. No WebGPU validation errors in the console
7. Tab-hiding and re-showing does not crash the renderer
8. All GPU work runs in the worker thread — main thread never touches WebGPU

---

## What This Enables

Once the demo renders correctly:

- **Radiance cascades** can be integrated into the color pass (replace placeholder ambient with cascade apply) — see [radiance-cascades-impl](radiance-cascades-impl.md)
- **Edit protocol** can be layered on top (Stage 2 dirty tracking → incremental mesh and summary rebuilds) — see [edit-protocol](edit-protocol.md)
- **The demo IS the renderer** — no migration step from demo to production; it evolves in place

---

## See Also

- [gpu-chunk-pool](gpu-chunk-pool.md) — slot allocation, buffer layouts, mesh pool
- [pipeline-stages](pipeline-stages.md) — full stage spec; demo implements R-2 through R-5 (R-6 through R-8 deferred)
- [depth-prepass](depth-prepass.md) — depth prepass and Hi-Z raster optimization chain
- [edit-protocol](edit-protocol.md) — what to add when the demo needs runtime voxel edits
- [radiance-cascades-impl](radiance-cascades-impl.md) — cascade 0 integration into the color pass
- [ADR-0013](../adr/0013-full-webgpu-worker-pipeline.md) — the architectural decision for full WebGPU in the worker
