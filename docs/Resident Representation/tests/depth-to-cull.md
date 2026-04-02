# Test: Depth to Cull (R-2 → R-3 → R-4)

**Type:** spec
**Status:** current
**Date:** 2026-03-22

> Proves the depth-to-cull chain is consistent: depth values are valid after R-2, the Hi-Z pyramid preserves the conservative max-reduction property, and R-4 cull decisions never reject visible geometry.

---

## What This Tests

The depth-to-cull chain transforms rasterized depth into a culled draw list:

```
depth_texture (R-2) → hiz_pyramid (R-3) → indirect_draw_buf + draw_count (R-4)
```

If depth values are invalid, the pyramid is wrong. If the pyramid is wrong, culling is wrong. If culling is wrong, visible geometry disappears. This document defines the tests that prove each link preserves correctness and conservatism.

---

## Chain Link 1: R-2 → R-3

**Claim:** The depth_texture produced by R-2 contains valid depth values that R-3 can faithfully reduce into the Hi-Z pyramid.

### Preconditions (R-3 input contract)

| ID | What R-3 requires | How R-2 must satisfy it |
|---|---|---|
| L1-1 | `depth_texture` contains depth values in [0, 1] (standard Z) or [1, 0] (reversed-Z) | R-2 renders with `depthCompare: 'less'` or `'greater'`, clear value 1.0 or 0.0 respectively |
| L1-2 | `depth_texture` dimensions match current viewport | R-2 POST-3: format is `depth32float` with correct usage flags |
| L1-3 | No concurrent writes to `depth_texture` when R-3 begins | R-2 POST-5: pipeline ordering |

### Tests

```
T-L1-1: Depth value range
  Run R-2 on a scene with known geometry
  Read back depth_texture via staging buffer
  For each texel:
    Assert: value >= 0.0
    Assert: value <= 1.0
    Assert: value is finite (not NaN, not Inf)

T-L1-2: Clear value for empty pixels
  Run R-2 on a scene where some pixels have no geometry coverage
  For each uncovered texel:
    Assert: value == depthClearValue (1.0 for standard Z, 0.0 for reversed-Z)

T-L1-3: Depth ordering
  Place two opaque quads at known distances d_near < d_far along the same view ray
  Run R-2
  At the overlapping pixel:
    Assert: depth value corresponds to d_near (nearer surface wins)
    (Standard Z: depth_near < depth_far; Reversed-Z: depth_near > depth_far)
```

---

## Chain Link 2: R-3 Pyramid Correctness

**Claim:** The Hi-Z pyramid's max-reduction is correct: no mip texel is less than any of its 2x2 source texels.

### Postconditions Under Test (R-3)

| ID | Condition | Reference |
|---|---|---|
| POST-1 | Mip 0 is a faithful value copy of `depth_texture` | HZ-1 |
| POST-2 | Each texel at mip L >= 1 equals `max(four parent texels at mip L-1)` | HZ-2 |
| POST-5 | No texel at any mip level is less than any of its contributing source texels | HZ-5 |

### Tests

```
T-L2-1: Mip 0 fidelity
  Run R-2, then R-3
  Read back depth_texture and hiz_pyramid mip 0
  For each texel (x, y):
    Assert: hiz_pyramid[0][x][y] == depth_texture[x][y]

T-L2-2: Max-reduction correctness
  Run R-3 on a known depth pattern (e.g., checkerboard of 0.3 and 0.7)
  For each mip level L >= 1, for each texel (x, y):
    Let src = (x * 2, y * 2)
    Let d00 = hiz_pyramid[L-1][src.x][src.y]
    Let d10 = hiz_pyramid[L-1][min(src.x+1, prev_w-1)][src.y]
    Let d01 = hiz_pyramid[L-1][src.x][min(src.y+1, prev_h-1)]
    Let d11 = hiz_pyramid[L-1][min(src.x+1, prev_w-1)][min(src.y+1, prev_h-1)]
    Assert: hiz_pyramid[L][x][y] == max(d00, d10, d01, d11)

T-L2-3: Conservative property (no texel less than source)
  For each mip level L >= 1, for each texel (x, y):
    Assert: hiz_pyramid[L][x][y] >= hiz_pyramid[L-1][x*2][y*2]
    Assert: hiz_pyramid[L][x][y] >= hiz_pyramid[L-1][min(x*2+1, prev_w-1)][y*2]
    Assert: hiz_pyramid[L][x][y] >= hiz_pyramid[L-1][x*2][min(y*2+1, prev_h-1)]
    Assert: hiz_pyramid[L][x][y] >= hiz_pyramid[L-1][min(x*2+1, prev_w-1)][min(y*2+1, prev_h-1)]

T-L2-4: Global maximum at 1x1 mip
  Read back the 1x1 mip level
  Read back the full depth_texture
  Assert: hiz_pyramid[max_mip][0][0] == max(all depth_texture texels)

T-L2-5: Edge handling for odd dimensions
  Create a depth_texture with non-power-of-two dimensions (e.g., 1920x1080)
  Run R-3
  For each mip level with odd source dimensions:
    Verify edge texels that sample fewer than 4 unique parents
    still satisfy the conservative property (T-L2-3)
```

---

## Chain Link 3: R-3 → R-4

**Claim:** R-4's cull decisions are conservative: no visible chunk is culled, chunks fully behind occluders are culled, and edge cases (near plane, viewport boundary) are handled safely.

### Preconditions (R-4 input contract)

| ID | What R-4 requires | How R-3 must satisfy it |
|---|---|---|
| L3-1 | `hiz_pyramid` is fully built (all mip levels valid) | R-3 POST-3 |
| L3-2 | Max-reduction property holds at every mip level | R-3 POST-2, POST-5 |
| L3-3 | Mip 0 dimensions match `depth_texture` dimensions | R-3 POST-4 |

### Tests

```
T-L3-1: Fully occluded chunk is culled
  Place a large wall (full-chunk occluder) at z=10
  Place a smaller chunk fully behind the wall at z=20
  Camera at z=0, looking toward +z
  Run R-2, R-3, R-4
  Assert: the behind-wall chunk does NOT appear in chunk_visible_list
  Assert: the behind-wall chunk does NOT appear in indirect_draw_buf
  Assert: the wall chunk DOES appear in both

T-L3-2: Visible chunk is never culled (conservative guarantee)
  For 100 random camera positions and chunk placements:
    Determine ground-truth visibility (CPU ray-cast or analytic projection)
    Run R-2, R-3, R-4
    For each chunk that is ground-truth visible:
      Assert: chunk appears in chunk_visible_list or indirect_draw_buf
  (False positives — drawing hidden chunks — are acceptable.
   False negatives — culling visible chunks — are failures.)

T-L3-3: Near-plane straddle is kept
  Place a chunk whose AABB straddles the camera near plane
  (some corners behind the camera, some in front)
  Run R-2, R-3, R-4
  Assert: the chunk is NOT culled
  Assert: the chunk appears in chunk_visible_list

T-L3-4: Partially visible chunk is kept
  Place a chunk partially behind an occluder (AABB extends past the occluder edge)
  Run R-2, R-3, R-4
  Assert: the chunk is NOT culled
  (Conservative: the pyramid's max-depth for the chunk's screen rect includes
   the unoccluded portion, so min_depth <= sampled_max_depth)

T-L3-5: Frustum-only rejection
  Place a chunk entirely outside the camera frustum (e.g., behind the camera)
  Run R-2, R-3, R-4
  Assert: the chunk is culled in phase 1 (frustum test, before Hi-Z test)
  Assert: chunk does NOT appear in chunk_visible_list

T-L3-6: All chunks visible (no occluders)
  Place N chunks all visible with no occlusion between them
  Run R-2, R-3, R-4
  Assert: chunk_visible_count == N (all survive phase 1)
  Assert: draw_count accounts for all N chunks in indirect_draw_buf
```

---

## Full Chain Integration Test

```
T-FULL-1: Depth → Pyramid → Cull → Validate
  Input: a scene with 3 layers of chunks at z=10, z=20, z=30
  Camera at z=0, looking toward +z
  The z=10 layer fully occludes the z=20 layer
  The z=30 layer is partially visible around the z=10 layer edges

  Run R-2, R-3, R-4

  Assert: z=10 layer chunks all survive culling
  Assert: z=20 layer chunks are all culled (fully behind z=10)
  Assert: z=30 layer chunks that are partially visible survive culling
  Assert: z=30 layer chunks that are fully behind z=10 are culled
  Assert: draw_count == (z=10 count) + (visible z=30 count)
  Assert: no GPU validation errors throughout the chain
```

---

## Consistency Properties (Hold for Any Valid Depth Buffer)

```
P-1: For every texel at every mip level L >= 1:
  hiz_pyramid[L][x][y] >= max(contributing texels at mip L-1)

P-2: For every chunk that is geometrically visible (projects onto the viewport
  and is not fully occluded by nearer geometry):
  The chunk appears in chunk_visible_list after R-4 phase 1

P-3: For every chunk in chunk_visible_list:
  chunk_resident_flags[slot] == 1
  chunk_flags.is_empty == 0

P-4: For every chunk whose AABB has any projected corner with w <= 0
  (straddles near plane):
  The chunk is kept (not culled)

P-5: draw_count after R-4 <= total number of resident non-empty chunks
  (culling can only reduce, never increase, the draw count)
```

These properties bridge R-2 postconditions (DEP POST-1 through POST-5), R-3 postconditions (HZ POST-1 through POST-6), and R-4 postconditions (CUL POST-1 through POST-8).

---

## See Also

- [R-2-depth-prepass](../stages/R-2-depth-prepass.md) -- producer of depth_texture
- [R-3-hiz-build](../stages/R-3-hiz-build.md) -- Hi-Z pyramid construction
- [R-4-occlusion-cull](../stages/R-4-occlusion-cull.md) -- consumer of Hi-Z pyramid, producer of indirect_draw_buf
- [depth-texture](../data/depth-texture.md) -- depth buffer format and invariants
- [hiz-pyramid](../data/hiz-pyramid.md) -- pyramid layout and max-reduction contract
- [indirect-draw-buf](../data/indirect-draw-buf.md) -- R-4 output consumed by R-5
