# Hi-Z Pyramid

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Per-frame transient — rebuilt every frame from depth_texture.

> Hierarchical depth pyramid for conservative occlusion testing. Each mip level holds the maximum depth of a 2x2 region from the level below.

---

## Identity

- **Texture name:** `hiz_pyramid`
- **WGSL type:** `texture_2d<f32>` (with mip levels)
- **Format:** `r32float`
- **Size:** mip 0 matches `depth_texture` dimensions; full mip chain down to 1x1
- **GPU usage:** `STORAGE_BINDING | TEXTURE_BINDING`
- **Binding:** `STORAGE_BINDING` for R-3 compute writes (per-mip); `TEXTURE_BINDING` for R-4 compute reads (with sampler or `textureLoad` at selected mip)

---

## Layout

A 2D texture with a full mip chain. Mip 0 has the same dimensions as `depth_texture`. Each subsequent mip level is half the width and height (rounded up) of the previous.

```
hiz_pyramid: texture_2d<f32>
  mip 0:  W × H          (same as depth_texture)
  mip 1:  ceil(W/2) × ceil(H/2)
  mip 2:  ceil(W/4) × ceil(H/4)
  ...
  mip N:  1 × 1

  mip_count = floor(log2(max(W, H))) + 1
```

### Reduction Rule

Each texel at mip level L is the **maximum** of the four corresponding texels at mip level L-1:

```
hiz[L][x, y] = max(
    hiz[L-1][2x,   2y],
    hiz[L-1][2x+1, 2y],
    hiz[L-1][2x,   2y+1],
    hiz[L-1][2x+1, 2y+1]
)
```

For texels at the edge where `2x+1` or `2y+1` would be out of bounds (odd-dimension levels), the reduction uses only the valid texels. The result is still the max of whatever texels exist.

### Why max-depth (not min-depth)

The pyramid answers: "What is the farthest depth in this screen region?" If a chunk's nearest projected depth is farther than the farthest known depth in the region it covers, the chunk is guaranteed to be fully behind existing geometry. Max-depth is the conservative choice for occlusion culling — it never incorrectly culls visible geometry.

### Total Memory

The full mip chain occupies approximately 1.33x the size of the base level:

```
total_texels = W * H * (1 + 1/4 + 1/16 + ...) ≈ W * H * 4/3
total_bytes  = total_texels * 4   (r32float = 4 bytes per texel)
```

At 1920x1080: ~10.6 MB. At 3840x2160: ~42.5 MB.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| HZ-1 | Mip 0 is a faithful copy of `depth_texture` (converted from depth32float to r32float) | R-3 first dispatch reads depth_texture, writes mip 0 |
| HZ-2 | Each texel at mip L >= 1 is the max of its 2x2 parent texels at mip L-1 | R-3 reduction kernel |
| HZ-3 | The pyramid is fully built (all mip levels written) before R-4 reads it | Pipeline barrier between R-3 and R-4 |
| HZ-4 | Dimensions of mip 0 match current depth_texture dimensions | Resize handler recreates both textures together |
| HZ-5 | Reduction is conservative — never produces a value less than any contributing texel | Max operation; no averaging or filtering |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| Each texel (any mip) | `0.0 .. 1.0` (NDC depth) | Matches depth_texture value domain |
| Mip level index | `0 .. mip_count - 1` | mip_count = floor(log2(max(W, H))) + 1 |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Hi-Z build pass (R-3) | Every frame, after R-2 | Full mip chain from depth_texture |

R-3 is a compute shader. Implementation options:

- **Multi-pass:** One dispatch per mip level, each reading the previous level. Simple, correct, requires barrier between each level.
- **Single-pass with shared memory:** One dispatch builds multiple mip levels using workgroup shared memory. Fewer barriers, more complex shader. Prefer multi-pass initially.

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Chunk coarse cull | R-4 phase 1 | Projects chunk AABB to screen rect, selects mip level that covers rect in ~4 texels, samples max depth, compares against chunk's min projected depth |
| Meshlet fine cull | R-4 phase 2 | Same technique at meshlet AABB granularity |
| Debug visualization | R-9 | Hi-Z mip level visualization mode — displays selected mip level as grayscale |

### Mip Level Selection for Occlusion Test

For an AABB projected to a screen-space rectangle of size (w_px, h_px) pixels:

```
mip_level = ceil(log2(max(w_px, h_px)))
```

This selects the coarsest mip level where the rectangle covers at most ~4 texels, ensuring a small number of texture reads per occlusion test. The test then samples 1-4 texels and takes the max.

---

## Lifecycle

### Creation

```typescript
const hizPyramid = device.createTexture({
  size: [canvas.width, canvas.height],
  format: 'r32float',
  mipLevelCount: Math.floor(Math.log2(Math.max(canvas.width, canvas.height))) + 1,
  usage: GPUTextureUsage.STORAGE_BINDING | GPUTextureUsage.TEXTURE_BINDING,
});
```

### Resize

Recreated whenever `depth_texture` is recreated (viewport resize). All bind groups referencing individual mip views must be rebuilt.

### Per-frame

Fully overwritten by R-3 every frame. No explicit invalidation needed between frames.

---

## Testing Strategy

### Unit tests (CPU-side)

1. **Mip count calculation:** For various (W, H), verify `floor(log2(max(W, H))) + 1` produces the correct number of mip levels.
2. **Mip dimension calculation:** For each mip level, verify `ceil(W / 2^L) x ceil(H / 2^L)` matches expected dimensions.

### GPU validation (WGSL compute)

3. **Reduction correctness:** Write a known depth pattern to depth_texture, run R-3, read back each mip level, verify every texel is the max of its 2x2 parent region.
4. **Conservative property:** For each texel at mip L, verify its value >= every contributing texel at mip L-1 (i.e., max was correctly applied, not min or average).
5. **Edge handling:** For odd-dimension mip levels, verify edge texels that have fewer than 4 parents still produce the correct max.
6. **Full chain:** Verify the 1x1 mip level contains the global maximum depth value in the scene.

### Integration tests

7. **Occlusion test correctness:** Place a large occluder, place a chunk fully behind it, verify R-4 occlusion test using the pyramid correctly culls the hidden chunk.
8. **No false negatives:** Place a chunk partially visible, verify R-4 does NOT cull it (conservative bias).

---

## See Also

- [depth-texture](depth-texture.md) — source texture that the pyramid is built from
- [indirect-draw-buf](indirect-draw-buf.md) — R-4 writes draw args for chunks surviving the Hi-Z test
- [depth-prepass](../depth-prepass.md) — Hi-Z pyramid is Tool 3 in the raster optimization chain
- [pipeline-stages](../pipeline-stages.md) — R-3 (Hi-Z build), R-4 (occlusion cull)
