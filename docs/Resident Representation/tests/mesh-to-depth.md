# Test: Mesh to Depth (R-1 → R-2)

**Type:** spec
**Status:** current
**Date:** 2026-03-22

> Proves the greedy mesher's output is valid depth-prepass input: draw_metadata offsets are in range, vertex data is well-formed, index values are bounded, and R-2 can render every non-empty chunk without GPU validation errors.

---

## What This Tests

The mesh-to-depth chain transforms chunk occupancy into a populated depth buffer:

```
Occupancy (R-1 input) → vertex/index pools + draw_metadata (R-1 output) → depth_texture (R-2 output)
```

R-1 is the sole producer of raster geometry. R-2 is the first consumer. If R-1 emits malformed geometry or invalid draw_metadata, R-2 will produce GPU validation errors, corrupt depth values, or silently skip chunks. This document defines the tests that prove the handoff is correct.

---

## Chain Link: R-1 → R-2

**Claim:** The greedy mesher's output (vertex_pool, index_pool, draw_metadata) is a valid input set for the depth prepass.

### Preconditions (R-2 input contract)

| ID | What R-2 requires | How R-1 must satisfy it |
|---|---|---|
| L1-1 | `vertex_pool` contains valid position data for vertex fetch | R-1 must write finite, in-range vec3f positions (POST-5: within chunk world extent) |
| L1-2 | `index_pool` contains valid triangle indices | R-1 must write indices where every value < vertex_count for the chunk (POST-3: index_count % 3 == 0) |
| L1-3 | `draw_metadata[slot].vertex_offset + vertex_count <= vertex_pool capacity` | R-1 POST-1 (DRW-1 pool bounds) |
| L1-4 | `draw_metadata[slot].index_offset + index_count <= index_pool capacity` | R-1 POST-2 (DRW-2 pool bounds) |
| L1-5 | `draw_metadata[slot].index_count % 3 == 0` | R-1 POST-3 (DRW-3 triangle validity) |
| L1-6 | Empty chunks produce zero-count draw_metadata, not absent entries | R-1 must write vertex_count=0, index_count=0 for empty slots (PRE-4 prevents enqueue, but metadata must be coherent) |

### Tests

```
T-L1-1: Vertex position validity
  Run R-1 on a chunk with known occupancy (e.g., single voxel at (10, 20, 30))
  Read back vertex_pool region [vertex_offset .. vertex_offset + vertex_count]
  For each vertex:
    Assert: position.x is finite (not NaN, not Inf)
    Assert: position.y is finite (not NaN, not Inf)
    Assert: position.z is finite (not NaN, not Inf)
    Assert: position falls within chunk world-space extent
      (chunk_origin.x <= pos.x <= chunk_origin.x + 64,
       chunk_origin.y <= pos.y <= chunk_origin.y + 64,
       chunk_origin.z <= pos.z <= chunk_origin.z + 64)

T-L1-2: Index range validity
  Run R-1 on a chunk with known occupancy
  Read back index_pool region [index_offset .. index_offset + index_count]
  For each index value idx:
    Assert: idx < vertex_count
  Assert: no index references a vertex outside the chunk's vertex_pool region

T-L1-3: Draw metadata vertex_offset in range
  For each rebuilt slot S:
    Assert: draw_metadata[S].vertex_offset + draw_metadata[S].vertex_count
            <= vertex_pool_capacity
  (Validates R-1 POST-1 / DRW-1)

T-L1-4: Draw metadata index_offset in range
  For each rebuilt slot S:
    Assert: draw_metadata[S].index_offset + draw_metadata[S].index_count
            <= index_pool_capacity
  (Validates R-1 POST-2 / DRW-2)

T-L1-5: Triangle count divisibility
  For each rebuilt slot S:
    Assert: draw_metadata[S].index_count % 3 == 0
  (Validates R-1 POST-3 / DRW-3)

T-L1-6: Empty chunk draw_metadata
  Run R-1 on a chunk with all-zero occupancy (should not be enqueued per PRE-4,
  but if draw_metadata[slot] is read for such a slot):
    Assert: draw_metadata[slot].vertex_count == 0
    Assert: draw_metadata[slot].index_count == 0
  This ensures R-2 skip logic (chunk_flags.is_empty check) is not the sole
  guard — the metadata itself is safe to consume even if the flag check fails.

T-L1-7: Region non-overlap across slots
  For any two resident rebuilt slots A and B (A != B):
    Assert: vertex regions [vertex_offset_A, vertex_offset_A + vertex_count_A)
            and [vertex_offset_B, vertex_offset_B + vertex_count_B) do not overlap
    Assert: index regions [index_offset_A, index_offset_A + index_count_A)
            and [index_offset_B, index_offset_B + index_count_B) do not overlap
  (Validates R-1 POST-9 / VP-1)
```

---

## GPU Integration: R-2 Renders R-1 Output

**Claim:** R-2 can render every non-empty chunk's mesh from R-1 without GPU validation errors.

### Tests

```
T-GPU-1: Depth prepass renders without validation errors
  Run R-1 on a scene with 10+ chunks of varied occupancy
  Run R-2 depth prepass consuming the produced vertex_pool, index_pool, draw_metadata
  Assert: no WebGPU validation errors
  Assert: no device lost events
  Assert: depth_texture is populated (not all clear value)

T-GPU-2: Single voxel depth correctness
  Place one voxel at a known world position
  Run R-1, then R-2
  Compute expected NDC depth for the voxel's front face given the camera
  Read back depth_texture at the voxel's projected screen position
  Assert: readback depth matches expected NDC depth within tolerance (1e-5)

T-GPU-3: Full chunk depth coverage
  Fill a chunk with all-occupied voxels (full cube)
  Run R-1, then R-2
  Read back depth_texture
  For each pixel in the chunk's screen projection:
    Assert: depth != clear_value (geometry was rasterized)

T-GPU-4: Multi-chunk consistency
  Build meshes for 4 chunks at different world positions
  Run R-2 with all 4 chunks' draw_metadata
  For each chunk:
    Assert: depth values appear at the expected screen regions
    Assert: nearer chunks produce smaller depth values than farther chunks
            at overlapping pixels

T-GPU-5: drawIndexed arguments are well-formed
  For each non-empty chunk's drawIndexed call:
    indexCount   = draw_metadata[slot].index_count      (validated by T-L1-5)
    firstIndex   = draw_metadata[slot].index_offset     (validated by T-L1-4)
    baseVertex   = draw_metadata[slot].vertex_offset    (validated by T-L1-3)
  Assert: the GPU executes each drawIndexed without out-of-bounds access
  (This is implicitly validated by T-GPU-1 — no validation errors.)
```

---

## Consistency Properties (Hold for Any Valid R-1 Output)

```
P-1: For every resident non-empty slot S:
  draw_metadata[S].vertex_offset + vertex_count <= vertex_pool_capacity
  draw_metadata[S].index_offset + index_count <= index_pool_capacity

P-2: For every resident non-empty slot S:
  All indices in index_pool[index_offset .. index_offset + index_count]
  are in range [0, vertex_count)

P-3: For every resident non-empty slot S:
  All vertex positions in vertex_pool[vertex_offset .. vertex_offset + vertex_count]
  are finite and within the chunk's world-space extent

P-4: For every resident non-empty slot S:
  draw_metadata[S].index_count % 3 == 0

P-5: R-2 consuming any draw_metadata that satisfies P-1..P-4 produces
  a valid depth_texture with no GPU validation errors
```

These properties bridge R-1 postconditions (POST-1 through POST-9) to R-2 preconditions (PRE-1 through PRE-3).

---

## See Also

- [R-1-mesh-rebuild](../stages/R-1-mesh-rebuild.md) -- producer stage: vertex/index pools and draw_metadata
- [R-2-depth-prepass](../stages/R-2-depth-prepass.md) -- consumer stage: depth-only render from mesh pools
- [vertex-pool](../data/vertex-pool.md) -- vertex buffer layout and packing convention
- [index-pool](../data/index-pool.md) -- companion index buffer
- [draw-metadata](../data/draw-metadata.md) -- per-slot draw offsets and counts
