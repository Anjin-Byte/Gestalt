# ADR-0011: Hybrid GPU-Driven Rendering Pipeline

**Type:** adr
**Status:** superseded
**Supersedes:** [ADR-0001](0001-renderer-choice.md), [ADR-0008](0008-design-gap-mitigations.md)
**Superseded by:** [ADR-0013](0013-full-webgpu-worker-pipeline.md)
**Date:** 2026-03-09
Depends on: ADR-0003 (Binary Greedy Meshing), ADR-0007 (Material Strategy), ADR-0010 (Radiance Cascades)

---

## Context

ADR-0001 selected Three.js with WebGPURenderer as the rendering backend.
This was the correct decision for early development: rapid prototyping,
automatic WebGL2 fallback, and a mature scene graph for debug visualization.

Since that decision, three features have been proposed that cannot be
implemented within Three.js's rendering abstraction:

1. **Radiance Cascades (ADR-0010)** — requires a depth prepass, compute
   passes between render stages, and a custom fragment shader for GI
   application.

2. **Hi-Z Occlusion Culling** (`docs/culling/hiz-occlusion-culling-report.md`)
   — requires an explicit depth target, a depth pyramid build via compute,
   a GPU culling pass, and indirect draw to avoid CPU round-trips.

3. **Fine-Grained Mesh Culling** — requires per-cluster AABBs, sub-object
   draw ranges driven by GPU, and potentially a visibility buffer.

All three share the same prerequisites:

| Prerequisite | ADR-0010 | Hi-Z Culling | Mesh Culling |
|-------------|----------|-------------|-------------|
| App-owned depth texture | Required | Required | Required |
| Compute dispatch between passes | Required | Required | Required |
| Indirect draw | Optional | Required | Required |
| Custom fragment shader | Required | Not needed | Optional |

Three.js provides none of these through its public API. Detailed evidence
is in `design/three-js-limits.md`.

### Alternatives Considered

**A. Bolt custom passes onto Three.js** — access `renderer.backend.device`
and run compute before/after Three.js's render call.

- Depth texture is not exposed and may not be sampleable
- No hook for compute between depth write and color draw
- Indirect draw requires replacing the draw loop
- Depends on Three.js internals that change across versions
- Amounts to writing a custom renderer coupled to Three.js's private API

**B. Hybrid: custom WebGPU for chunks, Three.js for everything else** — the
chunk rendering path (which accounts for >99% of triangles) moves to a
custom pipeline. Three.js renders debug helpers, UI, and non-chunk objects
as an overlay.

- Incremental migration (one phase at a time)
- Full control over the performance-critical path
- Three.js conveniences preserved for non-critical rendering
- Clean abstraction boundary at the ViewerBackend interface
- WebGL2 fallback unchanged

**C. Replace Three.js entirely** — custom WebGPU renderer for everything.

- Total control, cleanest architecture
- 4-6 weeks to reach feature parity
- Loses Three.js scene graph, helpers, camera controls
- Must implement material system, instanced rendering, resize handling
- No incremental migration path

---

## Decision

**Option B: Hybrid GPU-driven pipeline.**

The voxel chunk rendering path moves to a custom WebGPU pipeline that owns
the full pass sequence: depth prepass, Hi-Z pyramid, occlusion culling,
radiance cascade computation, and the main color pass with GI.

Three.js continues to render non-chunk objects (debug helpers, grid, axes,
bounding boxes, sprites, UI) as a final overlay pass.

---

## Rationale

### The convergence argument

Three features independently require the same three capabilities (depth
prepass, compute between passes, indirect draw). Implementing any one of
them requires the same architectural change. Building the infrastructure once
unblocks all three simultaneously.

### Incremental migration minimizes risk

The hybrid approach (see `design/hybrid-transition.md`) proceeds in 5 phases.
Each phase adds capability without removing the previous fallback. The testbed
continues to function at every step. Each phase has a debug panel toggle to
revert to the previous behavior.

### The abstraction boundary already exists

`ViewerBackend` in `apps/web/src/viewer/threeBackend.ts` returns a typed
interface. The custom pipeline extends this interface; it does not replace it.
The module system, data flow, and scene management are unaffected.

### Chunk rendering is the dominant workload

In a typical scene, chunk meshes account for >99% of triangles. Debug helpers,
grid, and UI sprites are negligible. Optimizing only the chunk path captures
nearly all the performance benefit of GPU-driven rendering.

---

## Consequences

### What changes

1. **`apps/web/src/viewer/threeBackend.ts`** — extended with `sharedDevice`,
   depth prepass, and custom pipeline dispatch
2. **New directory `apps/web/src/viewer/gpu/`** — custom WebGPU pipeline code:
   - `depthPrepass.ts` — depth-only render pass
   - `hizPyramid.ts` — depth pyramid build compute
   - `cullPass.ts` — occlusion culling compute
   - `cascadeBuild.ts` — radiance cascade compute
   - `chunkPipeline.ts` — main color pass with GI
   - `chunkBufferPool.ts` — global vertex/index buffer pool
   - `shaders/` — WGSL shader modules
3. **`apps/web/src/viewer/Viewer.ts`** — chunk meshes removed from Three.js
   scene (Phase 3+), routed to custom pipeline
4. **`apps/web/src/viewer/outputs.ts`** — chunk mesh output builds custom
   pipeline buffers instead of `THREE.Mesh` (Phase 3+)

### What does not change

- `crates/greedy_mesher/` — unchanged (produces same MeshOutput)
- `crates/voxelizer/` — unchanged
- `crates/wasm_*/` — unchanged
- Module system and `ModuleOutput` types — unchanged
- Camera controls (orbit, free camera) — unchanged (camera data shared)
- Debug panels, settings, animation loop — unchanged
- WebGL2 fallback path — unchanged (Three.js renders everything)
- All existing ADRs remain valid (this amends ADR-0001, does not replace it)

### ADR-0001 amendment

ADR-0001's decision — "Use Three.js with WebGPURenderer as the default
backend" — is narrowed:

- **Before:** Three.js renders all geometry
- **After:** Three.js renders non-chunk geometry (debug, UI, helpers).
  Chunk geometry renders through the custom WebGPU pipeline when WebGPU is
  available. When WebGPU is unavailable, Three.js renders everything (WebGL2
  fallback unchanged).

---

## See Also

- [`../philosophy.md`](../philosophy.md) — why the convergence is not accidental
- [`../design/three-js-limits.md`](../design/three-js-limits.md) — evidence for each limitation
- [`../design/pipeline-architecture.md`](../design/pipeline-architecture.md) — full frame pipeline
- [`../design/hybrid-transition.md`](../design/hybrid-transition.md) — incremental migration phases
- [`../spec/frame-graph.md`](../spec/frame-graph.md) — pass ordering and resource dependencies
- [`../spec/visibility-buffer.md`](../spec/visibility-buffer.md) — future meshlet/visibility buffer
- [`0001-renderer-choice.md`](0001-renderer-choice.md) — original decision (amended)
- [`0010-radiance-cascades.md`](0010-radiance-cascades.md) — radiance cascades (consumer)
- [`../../culling/hiz-occlusion-culling-report.md`](../../culling/hiz-occlusion-culling-report.md) — Hi-Z readiness report
