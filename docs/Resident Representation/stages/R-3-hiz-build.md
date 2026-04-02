# Stage R-3: Hi-Z Pyramid Build

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Stage type:** GPU compute
**Trigger:** Every frame, immediately after R-2 (depth prepass).

> Downsamples depth_texture into the hiz_pyramid mip chain. One dispatch per mip level. Each thread samples a 2x2 region from the previous level and outputs the maximum depth. Max-depth is the conservative choice for occlusion culling.

---

## Purpose

The Hi-Z pyramid is a hierarchical max-depth representation of the depth buffer. R-4 (occlusion cull) uses it to test chunk and meshlet AABBs against known depth at coarse granularity — a few texture reads per AABB instead of thousands of per-pixel depth comparisons. Without the pyramid, GPU-driven occlusion culling is not possible at interactive rates.

The pyramid must be rebuilt every frame because the depth buffer changes every frame (camera movement, chunk mesh updates).

---

## Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-1 | `depth_texture` contains valid depth for all visible geometry | R-2 postcondition (POST-1) |
| PRE-2 | `depth_texture` dimensions match current viewport | R-2 postcondition (POST-3) |
| PRE-3 | `hiz_pyramid` exists with correct dimensions and full mip chain | App lifecycle (creation/resize handler) |
| PRE-4 | R-2 has completed — no concurrent writes to `depth_texture` | Pipeline barrier / submission ordering |

---

## Inputs

| Texture | Access | Format | What's read |
|---|---|---|---|
| `depth_texture` | Read | `texture_depth_2d` / `depth32float` | Full-resolution depth from R-2. Read only at mip 0 generation. |
| `hiz_pyramid` mip L-1 | Read | `texture_2d<f32>` / `r32float` | Previous mip level. Read for generating mip L (L >= 2). |

---

## Transformation

### Mip 0: Copy from Depth Texture

The first dispatch reads `depth_texture` and writes `hiz_pyramid` mip 0. This is a format conversion from `depth32float` to `r32float` — the values are numerically identical, but the texture type changes to allow storage writes at individual mip levels.

```wgsl
@compute @workgroup_size(8, 8, 1)
fn build_mip0(@builtin(global_invocation_id) gid: vec3u) {
    let coords = vec2i(gid.xy);
    if coords.x >= i32(mip0_width) || coords.y >= i32(mip0_height) { return; }
    let depth = textureLoad(depth_texture, coords, 0);
    textureStore(hiz_mip0, coords, vec4f(depth, 0.0, 0.0, 0.0));
}
```

### Mip 1..N: Max-Depth Reduction

Each subsequent dispatch reads the previous mip level and writes the next. One thread per output texel:

```wgsl
@compute @workgroup_size(8, 8, 1)
fn build_mip(@builtin(global_invocation_id) gid: vec3u) {
    let out_coords = vec2i(gid.xy);
    if out_coords.x >= i32(mip_width) || out_coords.y >= i32(mip_height) { return; }

    let src = out_coords * 2;
    let d00 = textureLoad(hiz_prev_mip, src + vec2i(0, 0), 0).r;
    let d10 = textureLoad(hiz_prev_mip, src + vec2i(1, 0), 0).r;
    let d01 = textureLoad(hiz_prev_mip, src + vec2i(0, 1), 0).r;
    let d11 = textureLoad(hiz_prev_mip, src + vec2i(1, 1), 0).r;

    let max_depth = max(max(d00, d10), max(d01, d11));
    textureStore(hiz_current_mip, out_coords, vec4f(max_depth, 0.0, 0.0, 0.0));
}
```

### Edge Handling

For odd-dimension mip levels where `2x+1` or `2y+1` would sample out of bounds, the out-of-bounds texels are treated as having the same value as their in-bounds neighbor (clamped). `textureLoad` with out-of-bounds coordinates returns 0; to avoid this producing incorrect min values, the shader must clamp source coordinates to the previous level's valid range:

```wgsl
let src_max = vec2i(prev_mip_width - 1, prev_mip_height - 1);
let s00 = clamp(src + vec2i(0, 0), vec2i(0), src_max);
let s10 = clamp(src + vec2i(1, 0), vec2i(0), src_max);
let s01 = clamp(src + vec2i(0, 1), vec2i(0), src_max);
let s11 = clamp(src + vec2i(1, 1), vec2i(0), src_max);
```

Clamping is conservative: duplicating an edge texel into the max operation does not change the max (or raises it), which cannot produce false culling.

### Why Max-Depth (Not Min-Depth)

The pyramid answers: "What is the farthest depth in this screen region?" If a chunk's nearest projected depth is farther than the farthest known depth in the region it covers, the chunk is guaranteed to be entirely behind existing geometry. Max-depth never incorrectly culls visible geometry. Min-depth would answer a different question (closest depth) and could not be used for conservative occlusion rejection.

---

## Outputs

| Texture | Access | Format | What's written |
|---|---|---|---|
| `hiz_pyramid` mip 0 | Write | `r32float` | Faithful copy of `depth_texture` values |
| `hiz_pyramid` mip 1..N | Write | `r32float` | Max-depth reduction of previous mip level |

---

## Postconditions

| ID | Condition | Validates |
|---|---|---|
| POST-1 | Mip 0 is a faithful value copy of `depth_texture` | HZ-1 |
| POST-2 | Each texel at mip L >= 1 equals `max(four parent texels at mip L-1)` | HZ-2 (max-reduction correctness) |
| POST-3 | All mip levels are written (full chain down to 1x1) before R-4 begins | HZ-3 (pipeline ordering) |
| POST-4 | Mip 0 dimensions match `depth_texture` dimensions | HZ-4 (size consistency) |
| POST-5 | No texel at any mip level is less than any of its contributing source texels | HZ-5 (conservative property) |
| POST-6 | The 1x1 mip level contains the global maximum depth value in the scene | Reduction completeness |

---

## Dispatch

```
workgroup_size: (8, 8, 1)  — 64 threads per workgroup, one thread per output texel
```

**Mip 0:**
```
dispatch: (ceil(W / 8), ceil(H / 8), 1)
```

**Mip L (L >= 1):**
```
mip_width  = ceil(W / 2^L)
mip_height = ceil(H / 2^L)
dispatch: (ceil(mip_width / 8), ceil(mip_height / 8), 1)
```

**Total dispatches per frame:** `mip_count = floor(log2(max(W, H))) + 1`

At 1920x1080: 11 mip levels, 11 dispatches. Each dispatch is independent and requires a barrier against the previous (the output of mip L-1 must be visible before mip L reads it).

**Alternative — single-pass with shared memory:** A single dispatch builds multiple mip levels using workgroup shared memory, reducing barrier overhead. This is an optimization to pursue after the multi-pass approach is validated.

---

## Testing Strategy

### Unit tests (CPU-side)

1. **Mip count calculation:** For various (W, H), verify `floor(log2(max(W, H))) + 1` produces the correct number of mip levels.
2. **Mip dimension calculation:** For each mip level L, verify `ceil(W / 2^L) x ceil(H / 2^L)` matches expected dimensions.
3. **Edge-case dimensions:** Verify correct mip chain for non-power-of-two dimensions (e.g., 1920x1080, 1366x768).

### GPU validation (WGSL compute)

4. **Reduction correctness:** Write a known depth pattern to depth_texture, run R-3, read back each mip level, verify every texel is the max of its 2x2 parent region.
5. **Conservative property:** For each texel at mip L, verify its value >= every contributing texel at mip L-1.
6. **Edge handling:** For odd-dimension mip levels, verify edge texels that have fewer than 4 parents still produce the correct max.
7. **Full chain:** Verify the 1x1 mip level contains the global maximum depth value in the scene.
8. **Mip 0 fidelity:** Read back mip 0 and depth_texture, verify values are identical.

### Integration tests

9. **R-3 -> R-4 occlusion test:** Place a large occluder, place a chunk fully behind it, verify R-4 using the pyramid correctly culls the hidden chunk.
10. **No false negatives:** Place a chunk partially visible, verify R-4 does NOT cull it (conservative bias preserved through the pyramid).

---

## See Also

- [depth-texture](../data/depth-texture.md) — source texture (R-2 output) that the pyramid is built from
- [hiz-pyramid](../data/hiz-pyramid.md) — the texture written by this stage; layout, invariants, memory budget
- [indirect-draw-buf](../data/indirect-draw-buf.md) — R-4 writes draw args for chunks surviving the Hi-Z test
- [depth-prepass](../depth-prepass.md) — Hi-Z is Tool 3 in the raster optimization chain narrative
- [pipeline-stages](../pipeline-stages.md) — R-3 in the full stage diagram
