# Demo Renderer

**Type:** spec
**Status:** current
**Date:** 2026-03-21

Spec for the isolated GPU-resident demo module. Custom WebGPU renderer, no Three.js, own canvas.

The purpose of this module is validation: prove the resident architecture described in this vault works end-to-end before integrating it into the full application pipeline.

---

## Scope

This module is a contained experiment. It does not replace the existing Three.js pipeline. It does not share its canvas, device, or render state. It activates, renders, and deactivates independently.

**In scope for the initial demo:**
- Own WebGPU device and canvas lifecycle
- Hardcoded procedural test scene (no file I/O, no OBJ loading)
- GPU chunk pool (Option A layouts from [gpu-chunk-pool](gpu-chunk-pool.md))
- Depth prepass + Hi-Z build + occlusion cull + color pass (Stages R-2 through R-5)
- Orbit camera with keyboard/mouse input
- Cascade 0 only — deferred full cascade merge until [radiance-cascades-impl](radiance-cascades-impl.md) is written

**Out of scope for the initial demo:**
- Radiance cascade merge or GI (placeholder ambient only)
- Edit protocol / dirty tracking / runtime voxel edits
- Three.js overlay (no UI or debug rendering inside the demo canvas)
- Streaming or LRU eviction (fixed scene, all chunks resident)
- LOD / far-field approximations

---

## Module Location

```
apps/web/src/modules/gpuRendererDemo/
  index.ts              — module entry point implementing the module contract
  renderer.ts           — WebGPU device, canvas, and frame loop
  scene.ts              — procedural test scene (hardcoded chunks)
  pool.ts               — GPU chunk pool (slot allocation, buffer init)
  passes/
    depthPrepass.ts     — R-2: depth-only render pass
    hizBuild.ts         — R-3: Hi-Z pyramid compute
    occlusionCull.ts    — R-4: chunk AABB cull compute
    colorPass.ts        — R-5: main color render pass
    meshRebuild.ts      — greedy mesh dispatch for initial scene build
    summaryRebuild.ts   — occupancy summary + chunk_flags + aabb for initial scene
  shaders/
    depth.wgsl           — vertex shader, depth-only
    color.wgsl           — vertex + fragment, material lookup + placeholder GI
    hiz_build.wgsl       — compute: max-depth pyramid downsample
    occlusion_cull.wgsl  — compute: AABB vs. Hi-Z test, write indirect draw args
    mesh_rebuild.wgsl    — compute: greedy mesh → vertex/index pool
    summary_rebuild.wgsl — compute: occupancy_summary, chunk_flags, aabb
```

---

## Canvas Lifecycle

The demo module owns its canvas. It is created on activate, destroyed on deactivate. It does not share the main application canvas.

```typescript
// On activate:
const canvas = document.createElement('canvas');
canvas.width = container.clientWidth;
canvas.height = container.clientHeight;
container.appendChild(canvas);
const adapter = await navigator.gpu.requestAdapter();
const device = await adapter.requestDevice();
const context = canvas.getContext('webgpu') as GPUCanvasContext;
context.configure({ device, format: navigator.gpu.getPreferredCanvasFormat() });

// On deactivate:
device.destroy();
canvas.remove();
```

**Firefox note:** `requestAdapter` must be called while the page is visible. The module must guard against activation while the tab is hidden. (See existing fix in the voxelizer module for the same issue.)

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

On activate, before the first frame:

```
1. Allocate chunk pool buffers (see [gpu-chunk-pool](gpu-chunk-pool.md), Option A layouts)
2. Populate chunk_occupancy_atlas for all resident chunks (CPU writeBuffer)
3. Populate chunk_coord, chunk_slot_table_gpu (CPU writeBuffer)
4. Dispatch summary_rebuild for all resident chunks (compute)
5. Dispatch mesh_rebuild for all resident chunks (compute)
6. Wait for completion (onSubmittedWorkDone) before rendering first frame
```

No incremental rebuild on first frame — do a full blocking init pass.

---

## Frame Loop

```
Each frame:

1. Update camera uniforms (view, proj, camera_pos, camera_forward)

2. CPU sort: order chunk draw_metadata by dot(chunk_center - camera_pos, camera_forward)
   Write sorted indirect draw args to indirect_draw_buf_sorted (staging for R-4 input)

3. Stage R-2: Depth prepass
   Render all resident (non-empty) chunks to depth_texture, no fragment shader

4. Stage R-3: Hi-Z build
   Dispatch hiz_build.wgsl: downsample depth_texture → hiz_pyramid (full mip chain)

5. Stage R-4: Occlusion cull
   Dispatch occlusion_cull.wgsl: test each chunk AABB vs. hiz_pyramid
   Write surviving draw calls to indirect_draw_buf

6. Stage R-5: Color pass
   Draw surviving chunks via indirect_draw_buf
   Fragment shader: material lookup + placeholder ambient (emissive tint on emissive chunks)

7. Present
```

No edit protocol, no dirty propagation, no rebuild work after init.

---

## Camera

Orbit camera. Mouse drag to orbit, scroll to zoom, keyboard WASD to pan.

```typescript
// Camera state
let theta = 0;      // azimuth (radians)
let phi = Math.PI / 4;  // elevation (radians)
let radius = 200;   // distance from target
let target = [256, 128, 256];  // center of room

// View matrix from spherical coords
const eye = [
  target[0] + radius * Math.cos(phi) * Math.sin(theta),
  target[1] + radius * Math.sin(phi),
  target[2] + radius * Math.cos(phi) * Math.cos(theta),
];
```

Default position: outside and above the room, looking inward at the central sphere.

---

## Shader Notes

### depth.wgsl

Vertex-only pass. Transform chunk vertices using `view_proj` uniform. No fragment stage (or an empty fragment that outputs nothing). Writes depth only.

### color.wgsl

Vertex: same transform as depth.wgsl.

Fragment:
- Reconstruct world position from vertex output
- Look up material ID from vertex attribute (passed from vertex buffer)
- Look up material color from palette buffer
- Ambient term: `ambient_color * material_albedo`
- Emissive override: if `is_emissive` flag set on material, output emissive color directly

This is a placeholder. The GI integration (cascade apply pass) slots in here when [radiance-cascades-impl](radiance-cascades-impl.md) is complete.

### hiz_build.wgsl

One compute dispatch per mip level. Each workgroup samples 2×2 texels from the previous level and outputs the maximum depth value. Max-depth (not min-depth) is correct for occlusion queries: a chunk is culled only if its minimum projected depth is greater than the maximum depth in its screen footprint.

### occlusion_cull.wgsl

One thread per chunk. Projects chunk AABB corners to clip space, computes screen-space bounding rectangle, selects Hi-Z mip level covering the rectangle in ~4 texels, samples pyramid. If `chunk_min_depth > sampled_max_depth`, cull. Otherwise, write draw call to indirect output buffer via `atomicAdd` counter.

Conservative bias: if AABB straddles the near plane or projects outside NDC, always keep the chunk.

---

## Greedy Mesh Dispatch

Initial mesh build runs as a compute pass:

```wgsl
// mesh_rebuild.wgsl
@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let slot = queue_entry; // one dispatch per chunk in mesh_rebuild_queue
  // ... greedy mesh algorithm over chunk_occupancy_atlas[slot] ...
  // write to vertex_pool[draw_metadata[slot].vertex_offset ...]
  // write draw_metadata[slot].vertex_count, index_count
}
```

For the demo, greedy meshing runs once at init over all resident chunks. The algorithm is the same binary greedy mesh used by the existing Rust greedy mesher — ported to WGSL.

See [pipeline-stages](pipeline-stages.md) Stage R-1 for the full mesh rebuild stage spec.

---

## Module Contract

The demo module implements the same module contract as the existing modules (VoxelChunkPipelineModule, etc.) so it can be activated/deactivated from the existing module switcher.

```typescript
export class GpuRendererDemoModule implements AppModule {
  name = 'gpu-renderer-demo';

  async activate(container: HTMLElement): Promise<void> {
    // create canvas, init device, build scene, start frame loop
  }

  deactivate(): void {
    // stop frame loop, destroy device, remove canvas
  }
}
```

---

## Validation Criteria

The demo is complete when:

1. Scene renders correctly at 60fps on a desktop WebGPU-capable browser
2. Depth prepass populates `depth_texture` visibly correctly (debug view: render depth as grayscale)
3. Hi-Z pyramid visibly correct at each mip level (debug view option)
4. Occlusion cull removes expected chunks (chunks behind the central sphere culled from default view)
5. Draw call count drops measurably when camera faces the sphere vs. away from it
6. No WebGPU validation errors in the console
7. Tab-hiding and re-showing does not crash the renderer (Firefox `requestAdapter` guard)

---

## What This Enables

Once the demo renders correctly:

- **Radiance cascades** can be integrated into the color pass (replace placeholder ambient with cascade apply) — see [radiance-cascades-impl](radiance-cascades-impl.md)
- **Edit protocol** can be layered on top (Stage 2 dirty tracking → incremental mesh and summary rebuilds) — see [edit-protocol](edit-protocol.md)
- **Migration to full pipeline** becomes a matter of wiring the demo's GPU resources into the main application's module system rather than a standalone canvas

---

## See Also

- [gpu-chunk-pool](gpu-chunk-pool.md) — slot allocation, buffer layouts, mesh pool; this module implements Option A of both
- [pipeline-stages](pipeline-stages.md) — full stage spec; demo implements R-2 through R-5 (R-6 through R-8 deferred)
- [depth-prepass](depth-prepass.md) — depth prepass and Hi-Z raster optimization chain
- [edit-protocol](edit-protocol.md) — what to add when the demo needs runtime voxel edits
- [radiance-cascades-impl](radiance-cascades-impl.md) — next renderer document; cascade 0 integration into the demo color pass
