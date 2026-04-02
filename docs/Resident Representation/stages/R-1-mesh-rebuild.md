# Stage R-1: Greedy Mesh Rebuild

**Type:** spec
**Status:** current
**Date:** 2026-03-31
**Stage type:** GPU compute
**Trigger:** Dirty chunks in `mesh_rebuild_queue` (populated by compaction pass from `stale_mesh` bitset).

> Reads authoritative chunk occupancy, palette, index buffer, and palette metadata. Writes vertex/index geometry and draw metadata into the mesh pool. Binary greedy meshing with material-aware merge boundaries, ported to WGSL. Per-chunk dispatch.

---

## Purpose

Every chunk whose occupancy has changed since its last mesh build needs fresh surface geometry for rasterization. R-1 consumes the `mesh_rebuild_queue`, runs the binary greedy meshing algorithm on each dirty chunk's occupancy data, and writes packed vertex/index data into the shared mesh pools. This is the sole producer of raster geometry — downstream stages (R-2, R-4, R-5) read from the pools R-1 writes to.

Material identity is resolved per-voxel during the merge phase. Adjacent coplanar faces merge only if they share the same material. The resolved global MaterialId is emitted directly into the vertex buffer, eliminating per-fragment palette lookup in R-5.

---

## Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-1 | `chunk_occupancy_atlas[slot]` contains valid occupancy data | I-2 postcondition |
| PRE-2 | `chunk_palette_buf[slot]` contains valid palette entries with global MaterialIds | I-2 postcondition |
| PRE-3 | `chunk_index_buf[slot]` contains valid per-voxel palette indices, bitpacked at the bit width indicated by `palette_meta[slot]` | I-2 postcondition |
| PRE-4 | `palette_meta[slot]` contains valid `palette_size` (bits 0–15) and `bits_per_entry` (bits 16–23) | I-2 postcondition |
| PRE-5 | `chunk_flags[slot].stale_mesh == 1` for all slots in the rebuild queue | Compaction pass queued from edit-protocol |
| PRE-6 | `chunk_flags[slot].is_empty == 0` for all slots in the rebuild queue | Empty chunks are never enqueued for mesh rebuild |
| PRE-7 | `chunk_resident_flags[slot] == 1` for all slots being rebuilt | Pool manager |
| PRE-8 | `chunk_coord[slot]` contains the correct world-space chunk coordinate | I-2 postcondition |

---

## Inputs

| Buffer | Access | Per-slot size | What's read |
|---|---|---|---|
| `chunk_occupancy_atlas` | Read | 32 KB (8192 u32 words) | Full 64-cubed occupancy bitfield — the authoritative voxel truth for meshing |
| `chunk_palette_buf` | Read | Variable (≤512 B) | Palette entries (packed u16 MaterialIds) — resolved to global MaterialId per quad |
| `chunk_index_buf` | Read | Variable (≤256 KB) | Per-voxel palette indices, bitpacked at variable bit width |
| `palette_meta` | Read | 4 B | `palette_size` (u16) and `bits_per_entry` (u8) for decoding `chunk_index_buf` |
| `chunk_flags` | Read | 4 B | `stale_mesh` check to confirm slot needs rebuild; `is_empty` to skip |
| `chunk_coord[slot]` | Read | 16 B (vec4i) | World-space origin for computing vertex world positions |
| `mesh_rebuild_queue` | Read | 4 B per entry | Slot indices of chunks needing mesh rebuild |

---

## Transformation

For each slot in `mesh_rebuild_queue`:

### 1. Face Culling

For each of the six face directions, compute the face visibility bitmask across the usable 62x62 slice. A face is visible at voxel (x, y, z) when the voxel is occupied and its neighbor in the face direction is not.

This phase reads only the occupancy atlas. Material is not involved.

```
For each column (x, z) in the usable interior [1..62]:
  col     = occupancy[x * 64 + z] as u64
  neighbor = occupancy[neighbor_x * 64 + neighbor_z] as u64

  // +Y: occupied and y+1 is empty
  face_mask_pos_y = (col & !(col >> 1)) >> 1  // shift to usable range [0..61]

  // ±X, ±Z: occupied and lateral neighbor is empty
  face_mask_lateral = (col & !neighbor) >> 1
```

The result is a per-column u64 bitmask where bit y is set if that face is visible at usable coordinate y.

### 2. Visibility Bitmap Pre-Compute

For each slice (one thread per slice), scan the 62x62 grid and fill a private-memory bitmap marking all visible cells. This eliminates redundant face-mask reads during the merge loop.

```
for (primary, secondary) in 62x62:
  if face_visible(column_mask, y_bit):
    visible_bitmap[primary * 62 + secondary] = 1
```

This bitmap costs 121 u32 words (484 bytes) in private memory.

### 3. Material-Aware Greedy Merge

The merge loop sweeps the 62x62 grid, starting new quads at unprocessed visible cells and extending them in the primary (width) and secondary (height) directions.

**Material lookup at each candidate cell:**

```
fn read_material_id(slot, px, py, pz) -> u32:
  bpe         = (palette_meta[slot] >> 16) & 0xFF    // bits_per_entry
  voxel_index = px * 4096 + py * 64 + pz             // x-major flat index
  bit_offset  = voxel_index * bpe
  word_index  = bit_offset / 32
  bit_within  = bit_offset % 32
  mask        = (1 << bpe) - 1
  palette_idx = (index_buf[slot_base + word_index] >> bit_within) & mask
  pal_word    = palette_buf[slot_palette_base + (palette_idx >> 1)]
  material_id = (pal_word >> ((palette_idx & 1) * 16)) & 0xFFFF
  return material_id
```

The bitpacking decode is safe without cross-word handling because `bits_per_entry` is always a power of two that divides 32 (see IDX-1). See [material-aware-merge](../material-aware-merge.md) for the full rationale.

**Merge procedure:**

```
for each unprocessed visible cell at (primary, secondary):
  seed_mat = read_material_id(slot, px, pz, y)

  // Extend width: advance primary while visible AND same material
  width = 1
  while primary + width < 62:
    if !visible(primary + width, secondary): break
    if read_material_id(...) != seed_mat: break
    width++

  // Extend height: advance secondary while all cells in row are visible AND same material
  height = 1
  while secondary + height < 62:
    for each cell in [primary .. primary + width]:
      if !visible(cell, secondary + height): stop height
      if read_material_id(...) != seed_mat: stop height
    height++

  mark_processed(primary .. primary+width, secondary .. secondary+height)
  emit_quad(primary, secondary, width, height, seed_mat)
```

**Critical properties:**
- The seed material is determined by the first cell of the quad
- Width extension stops at the first material mismatch
- Height extension stops at the first row containing any material mismatch
- The `processed` bitmap prevents re-processing cells that were already merged

**Performance:** Material lookup costs 2 storage buffer reads per candidate cell (index_buf + palette_buf). The `palette_meta` read is hoisted out of the loop (one read per thread). See [material-aware-merge](../material-aware-merge.md) for the full cost analysis and the reasoning behind on-demand lookup vs. pre-computation.

### 4. Quad Emission

For each merged quad, emit four vertices and six indices:

```
// Vertex: vec3f position (12 B) + u32 normal_material (4 B)
// normal_material layout: [nx_snorm8, ny_snorm8, nz_snorm8, material_id_u8]

let nm = pack_normal_material(face_direction, seed_mat)
// seed_mat is the global MaterialId (u16), truncated to u8 for vertex packing

write_vertex(base + 0, corner0, nm)
write_vertex(base + 1, corner1, nm)
write_vertex(base + 2, corner2, nm)
write_vertex(base + 3, corner3, nm)

// Index pattern: CCW winding [0,1,2, 0,2,3]
write_indices([base, base+1, base+2, base, base+2, base+3])
```

The material_id in the vertex is the **global MaterialId**, not the palette index. This means R-5 can read `material_table[material_id]` directly without per-fragment palette lookup. The palette is only needed during meshing.

**Truncation to u8:** The vertex format packs material_id as a u8 (bits 31:24 of the normal_material word). This limits the addressable range to MaterialId 0–255. Since the palette protocol already limits per-chunk palette size to 256 entries, and each entry is a global MaterialId, the actual MaterialId values used within a chunk are bounded by the scene's material table. For scenes with more than 256 total materials, only 256 can be referenced per chunk. For scenes with global MaterialIds > 255, the u8 truncation would lose the upper bits. This is a known limitation accepted for Phase 2–4. Phase 5 or later may extend to u16 by repacking the vertex format.

### 5. Draw Metadata Write

Write the per-slot draw metadata:

```
draw_metadata[slot] = DrawMetadata {
    vertex_offset:  <start of this chunk's region in vertex_pool>,
    vertex_count:   <number of vertices emitted>,
    index_offset:   <start of this chunk's region in index_pool>,
    index_count:    <number of indices emitted>,
    material_base:  <palette offset for this chunk>,
    _padding:       0, 0, 0,
}
```

### 6. Flag Updates

After committing the new mesh:
- Stamp `mesh_version[slot] = chunk_version[slot]`
- Set `stale_meshlet[slot]` (triggers meshlet rebuild on the next compaction pass)
- Clear `stale_mesh[slot]` via the swap pass (not directly — follows the edit-protocol pattern)

---

## Outputs

| Buffer | Access | Per-slot size | What's written |
|---|---|---|---|
| `vertex_pool` | Write | Variable (up to `MAX_VERTS_PER_CHUNK * 16` B) | Packed position + normal + material vertices for all emitted quads |
| `index_pool` | Write | Variable (up to `MAX_INDICES_PER_CHUNK * 4` B) | Triangle indices referencing vertex_pool |
| `draw_metadata[slot]` | Write | 32 B | Vertex/index offsets, counts, material_base |
| `mesh_version[slot]` | Write | 4 B | Stamped to match `chunk_version[slot]` at rebuild time |
| `stale_meshlet[slot]` | Write | Bit | Set to indicate meshlet rebuild needed |

---

## Postconditions

| ID | Condition | Validates |
|---|---|---|
| POST-1 | `draw_metadata[slot].vertex_offset + vertex_count <= vertex_pool capacity` | DRW-1 (pool bounds) |
| POST-2 | `draw_metadata[slot].index_offset + index_count <= index_pool capacity` | DRW-2 (pool bounds) |
| POST-3 | `draw_metadata[slot].index_count % 3 == 0` | DRW-3 (triangle validity) |
| POST-4 | `mesh_version[slot] == chunk_version[slot]` | Version freshness |
| POST-5 | Every emitted vertex position falls within the chunk's world-space extent | Geometry correctness |
| POST-6 | Every emitted normal is axis-aligned (exactly one component is +/-1, others 0) | VP snorm8 encoding |
| POST-7 | Every emitted material_id is a valid index into `material_table` | Material correctness |
| POST-8 | No quad spans a material boundary — all voxels contributing to a quad share the same MaterialId | Merge correctness |
| POST-9 | `stale_meshlet[slot] == 1` after mesh commit | Triggers downstream meshlet rebuild |
| POST-10 | Vertex/index regions for this slot do not overlap with any other resident slot's regions | VP-1 (region isolation) |

---

## Dispatch

### Current Implementation (Phase 1–2)

```
workgroup_size: (64, 1, 1)  — one thread per slice
dispatch: (slot_count, 6, 1) — one workgroup per face direction per chunk

workgroup_id.x = chunk slot
workgroup_id.y = face direction (0=+Y, 1=-Y, 2=+X, 3=-X, 4=+Z, 5=-Z)
local_id.x     = slice index (0..61 active, 62-63 idle)
```

Each of the 62 active threads processes one slice of the 62x62 grid for one face direction of one chunk. Threads 62 and 63 return immediately (idle padding for workgroup alignment).

### Target Implementation (Phase 4+)

Queue-based dispatch with frame budget:
```
dispatch: indirect, from mesh_rebuild_queue count
budget: frame_budget uniform limits chunks per frame
```

---

## Bind Group Layout

```
@group(0) @binding(0) var<storage, read>       occupancy:      array<u32>;
@group(0) @binding(1) var<storage, read>       palette:        array<u32>;
@group(0) @binding(2) var<storage, read>       coord:          array<vec4i>;
@group(0) @binding(3) var<storage, read_write> vertex_pool:    array<u32>;
@group(0) @binding(4) var<storage, read_write> index_pool:     array<u32>;
@group(0) @binding(5) var<storage, read_write> draw_meta:      array<atomic<u32>>;
@group(0) @binding(6) var<storage, read>       index_buf_pool: array<u32>;
@group(0) @binding(7) var<storage, read>       palette_meta:   array<u32>;
```

Bindings 0–5 are existing (Phase 1). Bindings 6–7 are new for material-aware merge.

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Empty chunk:** All-zero occupancy produces `vertex_count == 0`, `index_count == 0`, draw_metadata with zero counts.
2. **Single voxel:** One occupied voxel produces exactly 6 faces * 2 triangles = 12 indices, 6 faces * 4 vertices = 24 vertices.
3. **Merged quads (same material):** Two adjacent same-material voxels sharing a coplanar face produce merged quads with fewer vertices than two independent voxels.
4. **Material boundary (different materials):** Two adjacent voxels with different materials do not merge — each produces independent quads on shared faces. This is the core F7 test.
5. **Full chunk (single material):** All-occupied single-material chunk produces only the 6 outer faces (all interior faces are culled). Merge proceeds maximally.
6. **Checkerboard materials:** Alternating two materials in a 3D checkerboard produces zero merging — each visible face is a 1x1 quad.
7. **Material region boundary:** A 4x4 slab where the left half is material A and the right half is material B. The +Y face should produce exactly two quads (2x4 and 2x4), not one merged 4x4 quad.

### Property tests (Rust, randomized)

8. **Region non-overlap:** For any two rebuilt slots, vertex_pool regions do not overlap.
9. **Index validity:** Every index in index_pool for a rebuilt slot is in range `[0, vertex_count)` relative to vertex_offset.
10. **Normal axis alignment:** All normals decode to exactly one axis-aligned unit vector.
11. **Capacity bound:** Vertex count never exceeds `MAX_VERTS_PER_CHUNK` for any randomized occupancy.
12. **Material consistency per quad:** Every vertex in a quad has the same material_id. No quad spans a material boundary.

### GPU validation

13. **CPU-GPU agreement:** Run greedy mesher on CPU and GPU for the same occupancy + palette + index_buf input, readback GPU results, compare vertex/index counts, geometry, and per-quad material_ids.
14. **Idempotency:** Running R-1 twice on the same input produces identical vertex_pool, index_pool, and draw_metadata.

### Cross-stage tests

15. **R-1 -> R-2:** After R-1 rebuilds a chunk, R-2 depth prepass renders correct depth for the new geometry.
16. **R-1 -> R-5:** After R-1, R-5 color pass renders correct per-material colors (each quad's material_id indexes correctly into material_table).
17. **R-1 -> R-4:** draw_metadata written by R-1 produces valid DrawIndexedIndirectArgs when consumed by R-4 fallback path.

---

## See Also

- [material-aware-merge](../material-aware-merge.md) — design rationale for on-demand lookup, register pressure analysis, cost model
- [chunk-index-buf](../data/chunk-index-buf.md) — bitpacking layout, invariants IDX-1 through IDX-5
- [chunk-palette](../data/chunk-palette.md) — palette layout, invariants PAL-1 through PAL-6
- [material-system](../material-system.md) — global material table, palette protocol
- [vertex-pool](../data/vertex-pool.md) — vertex buffer layout and packing convention
- [index-pool](../data/index-pool.md) — companion index buffer
- [draw-metadata](../data/draw-metadata.md) — per-slot draw offsets and counts
- [gpu-chunk-pool](../gpu-chunk-pool.md) — mesh pool design, buffer allocation
- [chunk-flags](../data/chunk-flags.md) — `stale_mesh` bit that triggers this stage
- [pipeline-stages](../pipeline-stages.md) — R-1 in the full stage diagram
