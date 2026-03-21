# Depth Prepass and Raster Optimization Chain

**Type:** spec
**Status:** current
**Date:** 2026-03-21

> The raster optimization path for Product 2 (surface structure / camera image).

These tools reduce the cost of drawing chunk geometry. They have no role in Product 1 (world-space ray work).

See [layer-model](layer-model.md) for why Product 2 and Product 1 optimizations must not be mixed.
See [pipeline-stages](pipeline-stages.md) for where each stage sits in the frame.

---

## The Problem

Greedy-meshed chunks produce triangle geometry. Those triangles go through the raster pipeline. In a dense voxel scene, many of those triangles will be:
- Behind other triangles (depth-occluded)
- Outside the frustum
- Submitting work that fragment shading will immediately discard

Fragment shading is the expensive part. Triangle setup and rasterization have non-trivial cost too, but the fragment shader — with material lookup, lighting, GI integration — is where the budget goes. The goal of raster optimization is to prevent fragment shading from running on geometry that won't contribute to the final image.

There are three tools, applied in this order of cheapness:

---

## Tool 1 — Front-to-Back Chunk Ordering

**Cost:** near zero. A sort on the CPU draw list.

**What it does:** If nearer chunks are drawn before farther chunks, the depth buffer fills with near geometry first. The GPU's early-Z test can then reject entire tiles of faraway geometry before their fragment shaders run.

**Why it works especially well for voxel worlds:** Chunk meshes are large and geometrically coherent. One nearby chunk can occlude many faraway chunks behind it. Sorting by chunk centroid distance from camera is sufficient — no per-triangle sort needed.

**Implementation:** Sort the `draw_metadata_buf` entries (or the indirect draw list) by `dot(chunk_center - camera_pos, camera_forward)` before writing `indirect_draw_buf`. This is O(N log N) on the CPU over the active chunk count.

**Limits:** Front-to-back ordering helps the depth test reject fragments, but it does not stop triangle setup and rasterization from running on invisible geometry. That is what Hi-Z adds.

---

## Tool 2 — Depth Prepass (Early-Z / Z-Prepass)

**Cost:** One additional render pass, depth-only, over all visible geometry.

**What it does:** Renders all chunk geometry with no fragment shader — depth writes only. The depth buffer is now fully populated before any fragment shading begins. The main color pass then runs with depth test enabled and depth writes disabled; any fragment that fails the depth test is rejected before its shader executes.

This is the "feed the depth buffer first so it can become a bouncer" strategy. The bouncer is at the door: fragments from farther geometry never enter the expensive shading path.

**Why this is a prerequisite for everything else:**
- Hi-Z pyramid is built from this depth buffer (Stage R-3)
- Radiance cascade probe placement reads world positions from this depth buffer (Stage R-6)
- Three.js overlay composites correctly against this depth buffer (Stage R-9)

The depth prepass is not just a raster optimization — it is the shared infrastructure prerequisite for the entire pipeline. It must be app-owned (not internal to Three.js) for downstream stages to read it. See [pipeline-stages](pipeline-stages.md) Stage R-2.

**GPU resource:** `depth_texture: depth32float`, app-owned `GPUTexture`. Written by R-2, read by R-3, R-5 (depth test), R-6, R-9.

---

## Tool 3 — Hi-Z Occlusion Culling

**Cost:** One compute pass to build the pyramid, one compute pass to test chunks.

**What it does:** Builds a depth pyramid (hierarchical-Z) from the depth texture. Tests each chunk's AABB against the pyramid. Chunks that are fully occluded behind known depth are removed from the draw list before any rasterization work is issued for them.

This is the tool that stops triangle setup from running on invisible chunks. Front-to-back ordering and the depth prepass together mean the depth buffer has near-accurate data when Hi-Z runs. Hi-Z uses that data to eliminate entire chunks before the GPU ever sees their triangles.

**The pyramid:** Each level of the pyramid is the maximum depth in a 2×2 tile of the level below (max-depth, not min-depth). A chunk AABB projects to a screen-space rectangle; the pyramid level that covers that rectangle in ~4 texels is sampled. If the chunk's minimum projected depth is greater than the sampled maximum pyramid depth, the chunk is entirely behind known geometry and can be culled.

**Conservative bias:** GPU rasterization has sub-pixel precision issues and Hi-Z sampling can produce false positives. Always bias slightly: if the test is ambiguous, keep the chunk. False negatives (culling a visible chunk) are corruption; false positives (drawing a hidden chunk) are just wasted work.

**GPU resources:** `hiz_pyramid: r32float` mipped 2D texture (Stage R-3). `indirect_draw_buf` written by cull compute (Stage R-4). See [pipeline-stages](pipeline-stages.md).

---

## What Hi-Z Is Not

Stated explicitly because the temptation to misuse it is real:

**Hi-Z is not a prefilter for world-space ray traversal.**

Hi-Z tells you which chunks the *camera* cannot see this frame. That is a correct and useful question for raster optimization. It is the wrong question for:
- GI probe rays (a probe behind the camera casts light forward)
- Radiance cascade world-space interval queries (intervals extend through occluded space)
- A&W traversal for shadows (an occluded chunk can still cast a shadow)
- Any Product 1 query

Using Hi-Z to gate Product 1 queries would produce incorrect lighting — specifically, light sources and emissive geometry that happen to be occluded from the camera would stop contributing to GI. This is a correctness bug, not a performance tradeoff.

The correct prefilter for Product 1 traversal is `chunk_flags.is_empty` — a world-space property, not a camera-space one. See [traversal-acceleration](traversal-acceleration.md).

---

## The Optimization Chain in Order

```
CPU sort: chunks front-to-back by distance
  ↓  (free early-Z rejection for sorted draw)

Stage R-2: Depth prepass
  All chunk geometry → depth_texture only
  No fragment shading
  ↓

Stage R-3: Hi-Z pyramid build
  depth_texture → hiz_pyramid (full mip chain, max-depth per cell)
  ↓

Stage R-4: Occlusion cull
  Each chunk AABB tested against hiz_pyramid
  Surviving chunks written to indirect_draw_buf
  ↓

Stage R-5: Main color pass
  Draws only surviving chunks via indirect draw
  Depth test against depth_texture (no depth write)
  Fragment shading runs only on visible, unoccluded geometry
```

Front-to-back sort is essentially free and should always be done.
Depth prepass is mandatory (prerequisite for cascade and overlay stages).
Hi-Z adds chunk-level culling on top — the draw list shrinks before rasterization.

---

## Implementation Notes

**Depth prepass and Three.js:**
The existing Three.js `WebGPURenderer` does not expose its internal depth texture for external use. The depth prepass must run as a custom WebGPU pass before Three.js renders. `renderer.backend.device` gives access to the raw `GPUDevice`; the prepass creates its own `GPURenderPassDescriptor` with the app-owned `depth_texture` as the depth attachment.

Three.js's own render call must be configured to reuse this depth texture rather than creating its own. Until the full hybrid pipeline from ADR-0011 is in place, this requires care: Three.js must not clear or overwrite the depth texture during its render.

**App-owned depth texture creation:**
```typescript
const depthTexture = device.createTexture({
  size: [canvas.width, canvas.height],
  format: 'depth32float',
  usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
});
```

`TEXTURE_BINDING` is required so that the Hi-Z compute shader and the cascade compute shader can read it. `depth32float` is preferred over `depth24plus` for precision at the far distances voxel scenes encounter.

**Resize handling:**
The depth texture must be recreated when the canvas is resized. All downstream consumers (Hi-Z pyramid, cascade atlas sizing, overlay) must be invalidated on resize.

---

## See Also

- [pipeline-stages](pipeline-stages.md) — full stage diagram; R-2 (depth prepass), R-3 (Hi-Z build), R-4 (cull)
- [layer-model](layer-model.md) — Product 2 (surface) vs Product 1 (world-space) — why Hi-Z stays on the raster side
- [traversal-acceleration](traversal-acceleration.md) — correct prefilter for world-space ray work (`chunk_flags.is_empty`)
- [ADR-0011](../adr/0011-hybrid-gpu-driven.md) — ADR-0011 hybrid pipeline (the broader migration context)
