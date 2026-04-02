# Draw Metadata

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Derived (produced by R-1 mesh rebuild pass).

> Per-slot offsets and counts into the vertex and index pools. Used by the depth prepass, occlusion cull, and color pass to issue indirect draws for chunk geometry.

---

## Identity

- **Buffer name:** `draw_metadata`
- **WGSL type:** `array<DrawMetadata>` (one struct per slot)
- **GPU usage:** `STORAGE`
- **Binding:** read by R-2 (depth prepass draw), R-4 (occlusion cull input / chunk-level fallback), R-5 (color pass draw)

---

## Layout

One slot occupies **32 bytes** (8 x `u32`, padded).

```wgsl
struct DrawMetadata {
    vertex_offset:  u32,   // byte 0..3   — start index into vertex_pool
    vertex_count:   u32,   // byte 4..7   — number of vertices
    index_offset:   u32,   // byte 8..11  — start index into index_pool
    index_count:    u32,   // byte 12..15 — number of indices
    material_base:  u32,   // byte 16..19 — offset into material palette for this chunk
    _padding_0:     u32,   // byte 20..23 — reserved, must be 0
    _padding_1:     u32,   // byte 24..27 — reserved, must be 0
    _padding_2:     u32,   // byte 28..31 — reserved, must be 0
};
```

Total struct size: 32 bytes (aligned to 16 bytes for GPU buffer access patterns).

```
For slot S:
  draw_metadata[S].vertex_offset  = offset into vertex_pool where this chunk's vertices begin
  draw_metadata[S].vertex_count   = number of vertices for this chunk
  draw_metadata[S].index_offset   = offset into index_pool where this chunk's indices begin
  draw_metadata[S].index_count    = number of indices for this chunk
  draw_metadata[S].material_base  = offset into the chunk's material palette
```

Total buffer size: `MAX_SLOTS * 32` bytes.

### Relationship to Pools

`vertex_offset` and `index_offset` are indices (not byte offsets) into `vertex_pool` and `index_pool` respectively. The vertex pool stores packed position + normal data; the index pool stores triangle indices.

A chunk with no geometry (empty chunk, or mesh not yet built) has `vertex_count == 0` and `index_count == 0`.

### Indirect Draw Generation

R-4 reads `draw_metadata` for surviving chunks and writes `DrawIndexedIndirectArgs` into `indirect_draw_buf`:

```
DrawIndexedIndirectArgs {
    index_count:    draw_metadata[slot].index_count,
    instance_count: 1,
    first_index:    draw_metadata[slot].index_offset,
    base_vertex:    draw_metadata[slot].vertex_offset,
    first_instance: slot,   // used by vertex shader to look up chunk_coord
}
```

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| DRW-1 | `vertex_offset + vertex_count <= vertex_pool capacity` | Mesh rebuild pass (R-1) allocation check |
| DRW-2 | `index_offset + index_count <= index_pool capacity` | Mesh rebuild pass (R-1) allocation check |
| DRW-3 | `index_count` is a multiple of 3 (triangles) | Greedy mesher output contract |
| DRW-4 | If `chunk_flags.is_empty == 1`, then `index_count == 0` | Mesh rebuild skips empty chunks; no geometry produced |
| DRW-5 | `vertex_count > 0` implies `index_count > 0` (vertices without indices are useless) | Greedy mesher always emits both or neither |
| DRW-6 | `mesh_version[slot]` matches `chunk_version[slot]` when draw_metadata is fresh | Mesh rebuild postcondition |
| DRW-7 | A slot with `chunk_resident_flags[slot] == 0` has undefined draw_metadata content | Pool lifecycle |
| DRW-8 | Padding fields are 0 | Mesh rebuild pass writes 0 to padding |
| DRW-9 | Total buffer size = `MAX_SLOTS * 32` bytes | Buffer creation |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| `vertex_offset` | `0 .. vertex_pool_capacity - 1` | Start of this chunk's vertex region |
| `vertex_count` | `0 .. MAX_VERTS_PER_CHUNK` | 0 for empty chunks |
| `index_offset` | `0 .. index_pool_capacity - 1` | Start of this chunk's index region |
| `index_count` | `0 .. MAX_INDICES_PER_CHUNK` | 0 for empty chunks; always divisible by 3 |
| `material_base` | `0 .. palette_size - 1` | Palette offset for this chunk |
| `slot_index` | `0 .. MAX_SLOTS-1` | Out-of-range = buffer overrun |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Mesh rebuild pass (R-1) | After greedy meshing a dirty chunk | Full 32 bytes per slot: offsets, counts, material_base |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Depth prepass draw | R-2 | vertex_offset, index_offset, index_count for depth-only render |
| Occlusion cull | R-4 phase 2 | Fallback chunk-level draw when meshlets are stale |
| Color pass draw | R-5 | vertex_offset, index_offset, index_count via indirect_draw_buf |
| Front-to-back sort | CPU (pre R-2) | Reads draw_metadata to build sorted draw list |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Struct layout:** Verify `DrawMetadata` is exactly 32 bytes with correct field offsets.
2. **Pool bounds:** After mesh rebuild, verify `vertex_offset + vertex_count <= vertex_pool capacity` and `index_offset + index_count <= index_pool capacity`.
3. **Triangle validity:** Verify `index_count % 3 == 0` for every slot with geometry.
4. **Empty chunk:** Slot with all-zero occupancy has `vertex_count == 0` and `index_count == 0`.
5. **Non-empty chunk:** Slot with occupied voxels has both `vertex_count > 0` and `index_count > 0`.

### Property tests (Rust, randomized)

6. **Non-overlap:** For any two resident slots A and B, verify their vertex regions `[vertex_offset, vertex_offset + vertex_count)` do not overlap, and likewise for index regions.
7. **Roundtrip:** Generate random occupancy, run greedy mesher, verify draw_metadata fields allow correct reconstruction of draw calls.
8. **Slot isolation:** Rebuilding mesh for slot N does not modify draw_metadata for slot M (where M != N).

### GPU validation (WGSL compute)

9. **Readback test:** After mesh rebuild, readback draw_metadata, verify fields match CPU-side greedy mesher output.
10. **Indirect draw test:** Build indirect draw args from draw_metadata, execute draw, verify rendered geometry matches expected output via readback of color/depth target.
