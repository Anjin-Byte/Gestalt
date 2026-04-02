# Depth Texture

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Per-frame transient — recreated every frame (and on viewport resize).

> The app-owned depth buffer. Written once per frame by the depth prepass (R-2), read by nearly every downstream stage. The critical sync point for the entire pipeline.

---

## Identity

- **Texture name:** `depth_texture`
- **WGSL type:** `texture_depth_2d`
- **Format:** `depth32float`
- **Size:** viewport width x viewport height (canvas dimensions)
- **GPU usage:** `RENDER_ATTACHMENT | TEXTURE_BINDING`
- **Binding:** varies per consumer — typically `@group(0) @binding(N)` as `texture_depth_2d` for compute reads, or as depth attachment for render passes

---

## Layout

A single 2D texture, one texel per screen pixel. Each texel stores a 32-bit floating-point depth value in normalized device coordinates (NDC).

```
depth_texture: texture_depth_2d
  width:  canvas.width   (physical pixels)
  height: canvas.height  (physical pixels)
  format: depth32float
  mip_levels: 1
  sample_count: 1
```

### Why depth32float

`depth32float` is chosen over `depth24plus` for two reasons:

1. **Precision at distance:** Voxel scenes with 64-cubed chunks spanning hundreds of chunk-lengths need far-plane precision. 32-bit float depth avoids z-fighting artifacts at the distances this pipeline encounters.
2. **Compute readability:** `depth32float` can be bound as `texture_depth_2d` for compute shader reads (Hi-Z build, cascade probe placement). `depth24plus` may not support `TEXTURE_BINDING` on all implementations.

### Why app-owned

The depth texture is **not** internal to Three.js or any framework renderer. It is created and managed by the application's custom WebGPU pipeline. This is required because:

- R-3 (Hi-Z build) must read it as a compute input
- R-6 (radiance cascade) must read it for probe world-position reconstruction
- R-9 (debug viz) must read it for depth visualization and wireframe depth test
- Three.js does not expose its internal depth texture for external use

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| DT-1 | Written by R-2 only within a frame. No other stage writes to it. | Render pass descriptor: R-2 has depthStoreOp=store; all later passes bind it read-only or as depth test (no write) |
| DT-2 | Contains valid depth for all visible geometry after R-2 completes | R-2 renders all chunk geometry with depth write enabled |
| DT-3 | Dimensions match current canvas physical pixel size | Resize handler recreates texture on dimension change |
| DT-4 | All downstream consumers must not begin until R-2 completes | Pipeline barrier / submission ordering |
| DT-5 | R-5 binds depth_texture for depth test with `depthWriteEnabled: false` | R-5 render pass descriptor |
| DT-6 | R-9 must not clear depth_texture — it reads existing depth | R-9 render pass uses `loadOp: load`, not `clear` |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| Each texel | `0.0 .. 1.0` (NDC depth) | 0.0 = near plane, 1.0 = far plane (reversed-Z inverts this convention — see camera setup) |
| Width | `1 .. device.limits.maxTextureDimension2D` | Typically canvas width in physical pixels |
| Height | `1 .. device.limits.maxTextureDimension2D` | Typically canvas height in physical pixels |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Depth prepass (R-2) | Every frame | Full-screen depth from all chunk geometry, depth-only (no color output) |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Hi-Z pyramid build | R-3 | Compute shader reads full texture to build mip chain |
| Occlusion cull (reference) | R-4 | Indirectly — via hiz_pyramid, which is derived from depth_texture |
| Main color pass | R-5 | Depth test (read-only, no write) to reject occluded fragments |
| Radiance cascade build | R-6 | Compute shader reads depth to reconstruct probe world positions via inverse projection |
| Cascade merge | R-7 | Bilateral depth weights for merge blending |
| Debug visualization | R-9 | Depth visualization mode; wireframe depth test; depth-aware compositing |

---

## Lifecycle

### Creation

```typescript
const depthTexture = device.createTexture({
  size: [canvas.width, canvas.height],
  format: 'depth32float',
  usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
});
```

`RENDER_ATTACHMENT` is required for R-2 to write it as a depth target.
`TEXTURE_BINDING` is required for R-3, R-6, R-7, R-9 to sample it in compute/fragment shaders.

### Resize

On viewport resize (canvas dimension change):

1. Destroy old `depth_texture`
2. Create new `depth_texture` at new dimensions
3. Invalidate and recreate `hiz_pyramid` (derived from depth_texture)
4. Invalidate cascade atlas sizing if probe density depends on viewport resolution
5. Update all bind groups that reference the texture

### Per-frame

R-2 clears and writes depth_texture at the start of each frame. No explicit invalidation needed between frames — R-2's `loadOp: clear` handles this.

---

## Testing Strategy

### Unit tests (TypeScript, CPU-side)

1. **Resize recreation:** Simulate canvas resize, verify depth_texture dimensions match new canvas size and old texture is destroyed.
2. **Usage flags:** Verify created texture has both `RENDER_ATTACHMENT` and `TEXTURE_BINDING` usage flags.

### GPU validation

3. **Readback after R-2:** Render a known scene (single cube at known position), read back depth texture via staging buffer, verify depth values match expected NDC depths for the cube's front face.
4. **Clear value:** After R-2 clear with no geometry, verify all texels contain the clear depth value (1.0 for standard, 0.0 for reversed-Z).
5. **No-write in R-5:** Render R-2 then R-5, read back depth texture — verify values are identical to post-R-2 readback (R-5 did not modify depth).

---

## See Also

- [hiz-pyramid](hiz-pyramid.md) — derived from depth_texture by R-3
- [indirect-draw-buf](indirect-draw-buf.md) — occlusion cull reads hiz_pyramid (derived from this texture)
- [depth-prepass](../depth-prepass.md) — R-2 stage that writes this texture
- [pipeline-stages](../pipeline-stages.md) — full stage diagram and read/write ownership
