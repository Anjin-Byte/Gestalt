# Test: Cull to Color (R-4 → R-5)

**Type:** spec
**Status:** current
**Date:** 2026-03-22

> Proves the occlusion culler's output is valid color-pass input: indirect draw buffer entries have valid index counts, pool references are in range, draw_count is bounded, and R-5 renders without GPU validation errors from the indirect buffer.

---

## What This Tests

The cull-to-color chain transforms the culled draw list into the final rendered image:

```
indirect_draw_buf + draw_count (R-4) → color_target (R-5)
```

R-4 writes `DrawIndexedIndirectArgs` entries via atomic append. R-5 consumes them via `drawIndexedIndirect`. If any entry has an out-of-range index count, invalid base vertex, or references non-existent pool regions, R-5 will produce GPU validation errors or render garbage. This document defines the tests that prove the handoff is correct.

---

## Chain Link: R-4 → R-5

**Claim:** Every entry in `indirect_draw_buf` written by R-4 is a valid `DrawIndexedIndirectArgs` that R-5 can safely consume.

### Preconditions (R-5 input contract)

| ID | What R-5 requires | How R-4 must satisfy it |
|---|---|---|
| L1-1 | `indexCount` is a multiple of 3 (complete triangles) | R-4 copies from draw_metadata or meshlet_desc, both of which produce triangle-aligned counts (R-1 POST-3) |
| L1-2 | `firstIndex + indexCount <= index_pool capacity` | R-4 POST-1 (ID-1): entries reference valid index_pool regions |
| L1-3 | `baseVertex` is a valid offset into vertex_pool | R-4 POST-1 (ID-1): entries reference valid vertex_pool regions |
| L1-4 | `draw_count <= MAX_DRAWS` | R-4 POST-5 (ID-7): atomic saturation guard |
| L1-5 | `instanceCount == 1` and `firstInstance == 0` for every entry | R-4 POST-2 (ID-2) |
| L1-6 | Every visible chunk from R-4 produces at least one draw call | No visible chunk is silently dropped between R-4 output and R-5 input |
| L1-7 | No duplicate draw calls for the same chunk (at chunk-level fallback granularity) | Atomic append is executed exactly once per surviving chunk or meshlet |

### Tests

```
T-L1-1: indexCount is a multiple of 3
  Run R-4 on a scene with varied chunk geometry
  Read back indirect_draw_buf (draw_count entries)
  For each entry i in [0, draw_count):
    Assert: indirect_draw_buf[i].indexCount % 3 == 0

T-L1-2: firstIndex + indexCount within index_pool bounds
  Read back indirect_draw_buf (draw_count entries)
  For each entry i in [0, draw_count):
    Assert: indirect_draw_buf[i].firstIndex + indirect_draw_buf[i].indexCount
            <= index_pool_capacity

T-L1-3: baseVertex within vertex_pool bounds
  Read back indirect_draw_buf (draw_count entries)
  For each entry i in [0, draw_count):
    Assert: indirect_draw_buf[i].baseVertex >= 0
    Assert: indirect_draw_buf[i].baseVertex < vertex_pool_capacity
    // Note: baseVertex is i32 in DrawIndexedIndirectArgs

T-L1-4: draw_count does not exceed MAX_DRAWS
  Run R-4 on a scene with many visible chunks (more than typical frame)
  Read back draw_count
  Assert: draw_count <= MAX_DRAWS

T-L1-5: instanceCount and firstInstance are fixed
  Read back indirect_draw_buf (draw_count entries)
  For each entry i in [0, draw_count):
    Assert: indirect_draw_buf[i].instanceCount == 1
    Assert: indirect_draw_buf[i].firstInstance == 0

T-L1-6: Every visible chunk produces a draw call
  Run R-4, record chunk_visible_list and chunk_visible_count
  Read back indirect_draw_buf and draw_count
  For each slot in chunk_visible_list:
    Assert: at least one entry in indirect_draw_buf references this slot's
            draw_metadata (chunk fallback) or meshlet_desc (meshlet path)
  Implementation note: for chunk fallback, match by firstIndex == draw_metadata[slot].index_offset
  and baseVertex == draw_metadata[slot].vertex_offset.

T-L1-7: No duplicate chunk-level draw calls
  Read back indirect_draw_buf (draw_count entries)
  Collect all entries that match chunk-level fallback pattern
    (i.e., indexCount == draw_metadata[slot].index_count for some slot)
  For each such slot:
    Assert: exactly one entry in indirect_draw_buf references that slot
  (Meshlet-level entries for the same chunk are expected to be multiple —
   one per surviving meshlet — but chunk-level fallback must be singular.)

T-L1-8: Saturation guard under pressure
  Set MAX_DRAWS to a small value (e.g., 16)
  Create a scene where R-4 would produce > 16 draw entries
  Run R-4
  Assert: draw_count == MAX_DRAWS (saturated, not overflowed)
  Assert: no out-of-bounds writes beyond indirect_draw_buf[MAX_DRAWS - 1]
  Assert: indirect_draw_buf[0 .. MAX_DRAWS] all contain valid entries
```

---

## GPU Integration: R-5 Renders R-4 Output

**Claim:** R-5 can consume every entry in `indirect_draw_buf` via `drawIndexedIndirect` without GPU validation errors and produce correct visual output.

### Tests

```
T-GPU-1: Color pass renders without validation errors
  Run full pipeline R-1 through R-5 on a scene with 10+ chunks
  Assert: no WebGPU validation errors during R-5
  Assert: no device lost events
  Assert: color_target contains non-clear-color pixels where geometry is visible

T-GPU-2: Visual correctness against reference
  Create a known scene: 3 chunks with distinct materials (red, green, blue)
  Run R-1 through R-5
  Read back color_target
  For each chunk's screen region:
    Assert: dominant color matches the assigned material albedo
            (within lighting tolerance)

T-GPU-3: Empty indirect buffer
  Set draw_count = 0 (no geometry survived culling)
  Run R-5
  Assert: color_target is entirely the clear color
  Assert: no GPU validation errors

T-GPU-4: Depth test rejection via R-2 depth
  Place two overlapping chunks: chunk A at z=10, chunk B at z=20
  Both survive R-4 culling (both are visible from some angle)
  R-2 depth buffer has z=10 depth at overlapping pixels
  Run R-5
  At overlapping pixels:
    Assert: color matches chunk A's material (nearer chunk wins via depth test)
    Assert: chunk B's fragments were rejected by early-Z

T-GPU-5: Fallback draw path correctness
  Force meshlet staleness for one chunk (meshlet_version != chunk_version)
  Run R-4 (produces chunk-level fallback entry for that chunk)
  Run R-5
  Assert: the fallback chunk renders correctly
  Assert: no visual gaps or missing geometry for the fallback chunk
```

---

## Consistency Properties (Hold for Any Valid R-4 Output)

```
P-1: For every entry in indirect_draw_buf[0 .. draw_count):
  indexCount % 3 == 0

P-2: For every entry in indirect_draw_buf[0 .. draw_count):
  firstIndex + indexCount <= index_pool_capacity

P-3: For every entry in indirect_draw_buf[0 .. draw_count):
  0 <= baseVertex < vertex_pool_capacity

P-4: draw_count <= MAX_DRAWS

P-5: For every slot in chunk_visible_list:
  At least one entry in indirect_draw_buf references geometry from that slot

P-6: No chunk-level fallback slot appears more than once in indirect_draw_buf

P-7: R-5 consuming any indirect_draw_buf that satisfies P-1..P-4 produces
  a valid color_target with no GPU validation errors
```

These properties bridge R-4 postconditions (POST-1 through POST-8) to R-5 preconditions (PRE-1 through PRE-8).

---

## See Also

- [R-4-occlusion-cull](../stages/R-4-occlusion-cull.md) -- producer of indirect_draw_buf and draw_count
- [R-5-color-pass](../stages/R-5-color-pass.md) -- consumer: renders via drawIndexedIndirect
- [indirect-draw-buf](../data/indirect-draw-buf.md) -- draw argument buffer layout and invariants
- [draw-metadata](../data/draw-metadata.md) -- chunk-level fallback source for R-4
- [vertex-pool](../data/vertex-pool.md) -- vertex data referenced by baseVertex
- [index-pool](../data/index-pool.md) -- index data referenced by firstIndex + indexCount
