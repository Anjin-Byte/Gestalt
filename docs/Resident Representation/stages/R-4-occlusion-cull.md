# Stage R-4: Occlusion Cull + Indirect Arg Write

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Stage type:** GPU compute
**Trigger:** Every frame, after R-3 (Hi-Z pyramid build).

> Two-phase GPU-driven culling. Phase 1 culls at chunk AABB granularity and writes a visible-chunk list. Phase 2 culls at meshlet AABB granularity (or falls back to chunk-level draw) and writes the final indirect draw arguments consumed by R-5.

---

## Purpose

R-4 reduces rasterization work by eliminating geometry that cannot contribute to the final image. Without R-4, every resident non-empty chunk submits all its triangles to the GPU — even chunks fully hidden behind nearer geometry. R-4 uses the Hi-Z pyramid (from R-3) to test each chunk's AABB against known depth, and when meshlets are available, further tests each meshlet's AABB for finer-grained rejection. The result is an `indirect_draw_buf` containing only the draw calls for visible geometry. Cost becomes proportional to visible surface, not resident surface.

---

## Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-1 | `hiz_pyramid` is fully built (all mip levels valid) | R-3 postcondition (POST-3) |
| PRE-2 | `chunk_aabb` contains valid world-space bounds for all resident non-empty chunks | I-3 postcondition |
| PRE-3 | `chunk_flags` is readable for all slots | I-3 postcondition |
| PRE-4 | `chunk_resident_flags` is readable for all slots | Pool manager |
| PRE-5 | `camera_uniform` contains current frame's view and projection matrices | App per-frame update |
| PRE-6 | `draw_metadata` is valid for all resident chunks with geometry | R-1 postcondition |
| PRE-7 | `meshlet_range_table` and `meshlet_desc_pool` are readable (may be stale — fallback handles this) | Meshlet build pass |

---

## Inputs

### Phase 1 — Chunk Coarse Cull

| Buffer / Texture | Access | Format | What's read |
|---|---|---|---|
| `chunk_aabb` | Read | `array<vec4f>` (2 per slot) | World-space tight AABB per chunk |
| `chunk_flags` | Read | `array<u32>` | `is_empty` bit — skip empty chunks early |
| `chunk_resident_flags` | Read | `array<u32>` | `is_resident` — skip non-resident slots |
| `hiz_pyramid` | Read | `texture_2d<f32>` (mipped, `r32float`) | Max-depth pyramid for occlusion testing |
| `camera_uniform` | Read | `mat4x4<f32>` x 2 | `view_proj` for AABB projection; frustum planes for frustum test |

### Phase 2 — Meshlet Fine Cull

| Buffer / Texture | Access | Format | What's read |
|---|---|---|---|
| `chunk_visible_list` | Read | `array<u32>` | Surviving slot indices from phase 1 |
| `meshlet_range_table` | Read | `array<MeshletRange>` (8 B per slot) | Per-slot meshlet start/count |
| `meshlet_desc_pool` | Read | `array<MeshletDesc>` (32 B per meshlet) | Meshlet AABB, index offset, vertex base |
| `meshlet_version` | Read | `array<u32>` | Freshness check per slot |
| `chunk_version` | Read | `array<u32>` | Ground-truth version for freshness comparison |
| `draw_metadata` | Read | `array<DrawMetadata>` (32 B per slot) | Fallback chunk-level draw when meshlets are stale |
| `hiz_pyramid` | Read | `texture_2d<f32>` (mipped) | Reused from phase 1 for meshlet-level Hi-Z test |
| `camera_uniform` | Read | `mat4x4<f32>` x 2 | Reused for meshlet AABB projection |

---

## Transformation

### Phase 1 — Chunk Coarse Cull

One thread per chunk slot. Tests each chunk against frustum and Hi-Z pyramid:

```wgsl
for slot in 0..N_SLOTS:
    if chunk_resident_flags[slot] == 0:          continue  // not loaded
    if (chunk_flags[slot] & IS_EMPTY_BIT) != 0:  continue  // no geometry

    let aabb_min = chunk_aabb[slot * 2 + 0].xyz;
    let aabb_max = chunk_aabb[slot * 2 + 1].xyz;

    // Frustum cull — test AABB against six frustum planes
    if frustum_cull(aabb_min, aabb_max, camera.frustum_planes): continue

    // Hi-Z occlusion cull
    if hiz_cull(aabb_min, aabb_max, camera.view_proj, hiz_pyramid): continue

    // Survived — append to visible list
    let idx = atomicAdd(&chunk_visible_count, 1u);
    chunk_visible_list[idx] = slot;
```

#### Hi-Z Cull Algorithm

For a given AABB:

1. **Project all 8 corners** to clip space via `view_proj * vec4f(corner, 1.0)`.
2. **Compute screen bounding rect** from the projected corners (in NDC, mapped to pixel coordinates).
3. **Compute chunk minimum projected depth** — the nearest corner's NDC depth (`min_z` of all projected corners).
4. **Select mip level:** `mip = ceil(log2(max(rect_width_px, rect_height_px)))` — the coarsest level where the rect covers at most ~4 texels.
5. **Sample pyramid:** Read 1-4 texels at the selected mip level covering the screen rect. Take the max of sampled values (`sampled_max_depth`).
6. **Compare:** If `chunk_min_depth > sampled_max_depth`, the chunk is entirely behind known geometry. Cull it.

#### Conservative Cases

- **Straddles near plane:** If any AABB corner has `w <= 0` after projection (behind or on the near plane), the chunk must be kept — it is too close to cull safely.
- **Outside NDC:** If the projected rect extends beyond screen bounds, clamp to viewport and keep (partial visibility).
- **Ambiguous test:** When in doubt, keep. False negatives (culling visible geometry) are corruption; false positives (drawing hidden geometry) are just wasted work.

### Phase 2 — Meshlet Fine Cull

Indirect dispatch sized by `chunk_visible_count` from phase 1. One workgroup per surviving chunk:

```wgsl
let slot = chunk_visible_list[workgroup_id.x];

// Freshness check — are meshlets up to date?
if meshlet_version[slot] != chunk_version[slot] {
    // Meshlets stale (rebuild in flight) — fall back to chunk-level draw
    let idx = atomicAdd(&draw_count, 1u);
    if idx < MAX_DRAWS {
        indirect_draw_buf[idx] = DrawIndexedIndirectArgs {
            index_count:    draw_metadata[slot].index_count,
            instance_count: 1u,
            first_index:    draw_metadata[slot].index_offset,
            base_vertex:    i32(draw_metadata[slot].vertex_offset),
            first_instance: 0u,
        };
    }
    return;
}

// Meshlets fresh — iterate and cull per meshlet
let range = meshlet_range_table[slot];
for m in range.start .. range.start + range.count {
    let desc = meshlet_desc_pool[m];

    // Frustum cull
    if frustum_cull(desc.aabb_min, desc.aabb_max, camera.frustum_planes): continue

    // Optional: backface cone cull (deferred — add when profiling shows benefit)
    // if backface_cone_cull(desc.normal_cone, camera.position): continue

    // Hi-Z occlusion cull
    if hiz_cull(desc.aabb_min, desc.aabb_max, camera.view_proj, hiz_pyramid): continue

    // Survived — emit draw call
    let idx = atomicAdd(&draw_count, 1u);
    if idx < MAX_DRAWS {
        indirect_draw_buf[idx] = DrawIndexedIndirectArgs {
            index_count:    desc.index_count,
            instance_count: 1u,
            first_index:    desc.index_offset,
            base_vertex:    i32(desc.vertex_base),
            first_instance: 0u,
        };
    }
}
```

The `draw_count` atomic counter is reset to 0 at the start of R-4 (before phase 1). Both the chunk fallback path and the meshlet path write to the same `indirect_draw_buf` and share the same atomic counter.

---

## Outputs

| Buffer | Access | Format | What's written |
|---|---|---|---|
| `chunk_visible_list` | Write (phase 1) | `array<u32>` | Slot indices of chunks surviving coarse cull |
| `chunk_visible_count` | Write (phase 1) | `u32` (atomic) | Number of surviving chunks; sizes phase 2 indirect dispatch |
| `indirect_draw_buf` | Write (phase 2) | `array<DrawIndexedIndirectArgs>` (20 B per entry) | One entry per surviving meshlet (or per chunk fallback) |
| `draw_count` | Write (phase 2) | `u32` (atomic) | Total number of draw entries; consumed by R-5 `multiDrawIndexedIndirect` |
| `visible_meshlet_count` | Write (phase 2) | `u32` (atomic) | Diagnostic counter for performance monitoring |

---

## Postconditions

| ID | Condition | Validates |
|---|---|---|
| POST-1 | Every entry in `indirect_draw_buf` references valid regions within `vertex_pool` and `index_pool` | ID-1 |
| POST-2 | `instance_count == 1` and `first_instance == 0` for every entry | ID-2 |
| POST-3 | `draw_count` accurately reflects the number of valid entries | ID-3 |
| POST-4 | No entry references a non-resident or empty chunk | ID-6 |
| POST-5 | `draw_count <= MAX_DRAWS` (atomic saturation guard) | ID-7 |
| POST-6 | No visible chunk was incorrectly culled (conservative guarantee) | Correctness — no false negatives |
| POST-7 | Chunks with stale meshlets are drawn at chunk-level granularity (fallback), not dropped | Meshlet freshness invariant |
| POST-8 | `indirect_draw_buf` is fully written before R-5 begins | ID-4 (pipeline barrier) |

---

## Dispatch

### Phase 1

```
workgroup_size: (64, 1, 1)  — one thread per slot
dispatch: (ceil(N_SLOTS / 64), 1, 1)
```

### Phase 2

```
workgroup_size: (64, 1, 1)  — one workgroup per visible chunk; threads iterate meshlets
dispatch: indirect via (chunk_visible_count, 1, 1)
```

Phase 2 uses `dispatchWorkgroupsIndirect` sized by `chunk_visible_count` from phase 1. A pipeline barrier between phase 1 and phase 2 ensures the visible list and count are committed.

---

## Testing Strategy

### Unit tests (CPU-side)

1. **Frustum cull:** AABB fully outside frustum returns culled. AABB intersecting frustum returns visible. AABB fully inside returns visible.
2. **Hi-Z cull logic:** Chunk with `min_depth > sampled_max_depth` is culled. Chunk with `min_depth <= sampled_max_depth` is kept.
3. **Near-plane straddle:** AABB crossing the near plane is always kept (conservative).
4. **Mip level selection:** For a given screen-space rect size, verify the selected mip level is `ceil(log2(max(w, h)))`.

### GPU validation (WGSL compute)

5. **Full occlusion:** Place a camera behind a large wall. Verify all chunks behind the wall are culled (`draw_count` reflects only chunks on the camera side).
6. **No false negatives:** Place a chunk partially visible behind an occluder. Verify it is NOT culled.
7. **Empty/non-resident skip:** Mark slots as empty or non-resident. Verify they never appear in `chunk_visible_list` or `indirect_draw_buf`.
8. **Fallback path:** Set `meshlet_version[slot] != chunk_version[slot]` for one chunk. Verify R-4 emits one chunk-level draw entry with correct `draw_metadata` offsets, not individual meshlet entries.
9. **Atomic counter accuracy:** Verify `draw_count` equals the actual number of entries written.

### Property tests (randomized)

10. **Conservative guarantee:** For 1000 random camera positions and chunk placements, verify no visible chunk (whose AABB intersects the frustum and is not fully occluded) is missing from `indirect_draw_buf`.
11. **Saturation guard:** Set `MAX_DRAWS` to a small value. Verify no out-of-bounds writes when more meshlets pass than `MAX_DRAWS` allows.

### Cross-stage tests

12. **R-4 -> R-5:** Verify R-5 `drawIndexedIndirect` successfully consumes the entries written by R-4 and produces correct rendered output.
13. **R-3 -> R-4:** Corrupt one mip level of `hiz_pyramid` (set all zeros). Verify R-4 keeps all chunks (conservative — zero max-depth means nothing is "farther than zero").

---

## See Also

- [hiz-pyramid](../data/hiz-pyramid.md) — the occlusion test data read by both phases
- [chunk-aabb](../data/chunk-aabb.md) — world-space bounds tested in phase 1
- [indirect-draw-buf](../data/indirect-draw-buf.md) — the draw argument buffer written by phase 2
- [draw-metadata](../data/draw-metadata.md) — chunk-level fallback draw parameters
- [chunk-flags](../data/chunk-flags.md) — `is_empty` and `is_resident` skip bits
- [meshlets](../meshlets.md) — two-phase cull design, meshlet descriptor, fallback behavior
- [depth-prepass](../depth-prepass.md) — raster optimization chain narrative (Hi-Z is Tool 3)
- [pipeline-stages](../pipeline-stages.md) — R-4 in the full stage diagram
