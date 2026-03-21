# Hybrid Transition Plan

Date: March 9, 2026

---

## Principle

The migration from Three.js-only rendering to the hybrid GPU-driven pipeline
is incremental. At every step, the existing testbed continues to function. No
feature is lost during migration. Each phase adds capability without removing
the previous fallback.

---

## Current State

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  Module       │────▶│  Viewer       │────▶│  Three.js    │
│  (outputs)    │     │  (scene mgmt) │     │  renderer    │
└──────────────┘     └──────────────┘     └──────────────┘
```

- Modules produce `ModuleOutput` (meshes, voxels, points, lines)
- Viewer converts outputs to Three.js objects (`outputs.ts`)
- `threeBackend.ts` calls `renderer.render(scene, camera)` once per frame
- Everything in one render call, one scene graph

---

## Phase 0: Shared GPU Device

**Goal:** Establish a single `GPUDevice` shared between Three.js and custom
compute work.

**What changes:**
1. Extract `GPUDevice` from `WebGPURenderer` at init time (already proven
   possible via `renderer.backend.device` in `webgpuDiagnostics.ts:12`)
2. Store as `sharedDevice` on `ViewerBackend`
3. Modules and custom pipeline use this device; no second `requestDevice()`
4. If `WebGLRenderer` is active, `sharedDevice` is `null` — all custom
   pipeline features gracefully disabled

**What doesn't change:** Rendering is still 100% Three.js. This phase only
establishes the device handle.

**Exit criteria:** `sharedDevice` accessible; compute shader can dispatch a
trivial workload using it.

---

## Phase 1: Depth Prepass

**Goal:** Render chunk depth to an app-owned texture before the main Three.js
render.

**What changes:**
1. Create `depth_texture` with `RENDER_ATTACHMENT | TEXTURE_BINDING`
2. Write a minimal depth-only render pass using raw WebGPU:
   - Create pipeline with vertex shader only (no fragment output)
   - Bind chunk mesh vertex/index buffers
   - Issue `drawIndexed` per chunk (CPU-driven, same as Three.js but explicit)
3. Three.js main render continues as before, but with depth test set to
   read the prepass result (or independently — overlay mode)
4. Debug visualization: display depth buffer as a fullscreen quad

**What doesn't change:** Three.js still renders all color. Chunks still use
`THREE.Mesh` objects. The depth prepass is an additional pass, not a
replacement.

**Unblocks:** Hi-Z pyramid build, radiance cascade depth queries.

**Exit criteria:** Depth texture matches Three.js-rendered depth. Compute
shader can sample it.

---

## Phase 2: Compute Infrastructure

**Goal:** Hi-Z pyramid build and radiance cascade compute passes run between
depth prepass and Three.js render.

**What changes:**
1. Implement Hi-Z pyramid as a chain of compute dispatches
   (see `docs/culling/hiz-occlusion-culling-report.md` §4)
2. Implement radiance cascade build as compute dispatches
   (see ADR-0010 Phase 2-3)
3. Upload chunk occupancy data as 3D texture for cascade raymarching
4. Frame ordering:
   ```
   depth prepass → hi-z pyramid → cascade build → Three.js render
   ```
5. Cascade output bound as a texture, readable in a custom `ShaderMaterial`
   on chunk meshes (Three.js can bind custom textures to materials)

**What doesn't change:** Draw calls still through Three.js. No indirect draw
yet. Culling compute runs but its output is not yet connected to draw
submission.

**Exit criteria:** Hi-Z pyramid correct (debug viz). Cascade produces visible
indirect lighting on chunks.

---

## Phase 3: Custom Color Pass

**Goal:** Replace Three.js's render of chunk meshes with a custom color pass
that uses GI data and supports indirect draw.

**What changes:**
1. Remove chunk `THREE.Mesh` objects from the Three.js scene
2. Build custom render pipeline:
   - Vertex shader: transform chunk vertices (same math as Three.js)
   - Fragment shader: material atlas lookup (ADR-0007) + GI from cascades
3. Issue draw calls from the custom pipeline (initially CPU-driven
   `drawIndexed`, same as Three.js but in custom pass)
4. Three.js continues to render non-chunk objects (helpers, debug, UI) as
   an overlay pass after the custom color pass
5. Frame ordering:
   ```
   depth prepass → hi-z → cascade → custom color pass → Three.js overlay
   ```

**What doesn't change:** Non-chunk rendering. Camera management. Module system.
Debug panels.

**Exit criteria:** Chunks render from custom pipeline with GI. Visual output
matches or exceeds Three.js path. Performance equal or better.

---

## Phase 4: Indirect Draw + GPU Culling

**Goal:** GPU decides which chunks to draw. CPU issues a single indirect draw.

**What changes:**
1. Allocate global vertex/index buffer pool (replaces per-chunk buffers)
2. Allocate `DrawIndexedIndirectArgs[]` buffer (one per chunk)
3. Hi-Z culling compute writes `instance_count = 0` for occluded chunks
4. Depth prepass uses previous frame's cull result (two-phase occlusion)
5. Custom color pass uses `drawIndexedIndirect` in a loop (or
   `multiDrawIndexedIndirect` if available)
6. CPU work per frame: upload dirty chunks + one `commandEncoder.finish()`

**What doesn't change:** Radiance cascades (already in custom pass). Three.js
overlay. Module system. Anything outside the viewer.

**Exit criteria:** Draw call count drops to 1 (or N for multi-draw). GPU
timings improve in occluded scenes. No visible artifacts.

---

## Phase 5: Fine-Grained Culling (Optional)

**Goal:** Sub-chunk culling for additional performance in complex scenes.

**What changes:**
1. Greedy mesher outputs face-direction groups as sub-ranges in index buffer
2. Per-direction AABB stored alongside chunk bounds
3. Backface direction culling (3 of 6 groups trivially rejected per chunk)
4. Hi-Z test per sub-range instead of per chunk
5. (Future) Meshlet clustering within face groups

**What doesn't change:** Everything from Phase 4 works at coarser granularity
as a fallback.

---

## Code Impact Per Phase

| Phase | Files modified | Files created | Risk |
|-------|---------------|--------------|------|
| 0 | `threeBackend.ts` | — | Minimal |
| 1 | `threeBackend.ts` | `viewer/gpu/depthPrepass.ts` | Low |
| 2 | `threeBackend.ts` | `viewer/gpu/hizPyramid.ts`, `viewer/gpu/cascadeBuild.ts` | Medium |
| 3 | `Viewer.ts`, `outputs.ts` | `viewer/gpu/chunkPipeline.ts`, `viewer/gpu/shaders/` | Medium-High |
| 4 | `viewer/gpu/chunkPipeline.ts` | `viewer/gpu/chunkBufferPool.ts`, `viewer/gpu/cullPass.ts` | Medium |
| 5 | Greedy mesher output format | `viewer/gpu/clusterCull.ts` | Low (additive) |

### ViewerBackend Interface Evolution

```typescript
// Phase 0
interface ViewerBackend {
  render(): void;
  resize(w: number, h: number): void;
  dispose(): void;
  readonly sharedDevice: GPUDevice | null;  // NEW
}

// Phase 1-2
interface ViewerBackend {
  render(): void;          // Now: depth prepass → compute → Three.js render
  resize(w: number, h: number): void;
  dispose(): void;
  readonly sharedDevice: GPUDevice | null;
  readonly depthTexture: GPUTexture | null;  // NEW
}

// Phase 3+
interface ViewerBackend {
  render(): void;          // Now: depth → compute → custom color → Three.js overlay
  resize(w: number, h: number): void;
  dispose(): void;
  readonly sharedDevice: GPUDevice | null;
  readonly depthTexture: GPUTexture | null;
  setChunkData(pool: ChunkBufferPool): void;  // NEW
  setGIEnabled(enabled: boolean): void;       // NEW
  setCullingEnabled(enabled: boolean): void;   // NEW
}
```

---

## Rollback Strategy

Each phase has a toggle in the debug panel. If a phase causes regressions:

1. **Phase 1:** Skip depth prepass, Three.js renders as before
2. **Phase 2:** Skip compute passes, no Hi-Z or GI (Three.js render only)
3. **Phase 3:** Re-add chunk meshes to Three.js scene, skip custom color pass
4. **Phase 4:** Fall back to CPU-driven drawIndexed in custom pass
5. **Phase 5:** Fall back to chunk-level culling (Phase 4)

The WebGL2 fallback path is never modified. It always renders through Three.js
without any custom pipeline features.

---

## See Also

- [`pipeline-architecture.md`](pipeline-architecture.md) — target state of the full pipeline
- [`three-js-limits.md`](three-js-limits.md) — why each phase is necessary
- [`../philosophy.md`](../philosophy.md) — the convergence argument
- [`../../greedy-meshing-docs/threejs-buffer-management.md`](../../greedy-meshing-docs/threejs-buffer-management.md) — current buffer system being replaced
