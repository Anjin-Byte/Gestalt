# Stage I-2: Chunk Upload

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Stage type:** CPU -> GPU transfer
**Trigger:** After I-1 (voxelization) completes, or after chunk data arrives from network/disk.

> Transfers authoritative chunk data from CPU to a GPU pool slot. The gate between producer output and the GPU-resident runtime.

---

## Purpose

The GPU chunk pool is the single source of truth for all rendering, traversal, and GI consumers. No consumer reads CPU-side chunk data directly. I-2 is the sole mechanism by which new or updated chunk data enters the GPU pool.

I-2 takes the `CompactVoxel[]` courier from I-1 (or equivalent data from network/disk) and writes it into a pool slot's occupancy atlas, palette buffer, and index buffer. After I-2 completes, the slot is marked resident and flagged for summary rebuild (I-3).

---

## Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-1 | `CompactVoxel[]` (or equivalent chunk data) is valid and complete for the target chunk | I-1 postcondition / network deserializer |
| PRE-2 | A free pool slot is available (from `free_slots` freelist) | Pool manager (CPU) |
| PRE-3 | `material_table` is populated for all MaterialIds referenced in the chunk's palette | Scene init / material system |
| PRE-4 | The GPU device is ready to accept `writeBuffer` commands | WebGPU device lifecycle |
| PRE-5 | No other upload or edit is concurrently targeting the same slot | Slot director (CPU) |

---

## Inputs

| Source | Format | What's read |
|---|---|---|
| `CompactVoxel[]` (CPU) | Rust struct array | Per-voxel position + material, grouped by chunk |
| CPU-side `opaque_mask: [u64; 4096]` | Bitpacked columns | 64x64x64 occupancy (built from CompactVoxel during ingest) |
| CPU-side `PaletteMaterials` | Palette + index buffer | Material palette and per-voxel palette indices |
| `ChunkCoord` | `(i32, i32, i32)` | World-space chunk coordinate |
| `free_slots` (CPU) | `Vec<SlotIndex>` | Available pool slot indices |

---

## Transformation

For each chunk being uploaded:

### 1. Slot Allocation

```
slot = free_slots.pop()
slot_table[chunk_coord] = slot
```

The slot directory is CPU-managed. The GPU does not allocate or free slots. See [gpu-chunk-pool](../gpu-chunk-pool.md) for slot lifecycle details.

### 2. Occupancy Atlas Write

```
device.writeBuffer(
    chunk_occupancy_atlas,
    offset = slot * 8192 * 4,    // 32 KB per slot
    data   = opaque_mask as [u32; 8192]
)
```

Writes the full 64x64x64 column-major bitpacked occupancy. Each column (x, z pair) is a u64 stored as two consecutive u32 words. The 1-voxel padding border (x, z = 0 or 63) contains neighbor boundary data if available, or zeros for newly loaded chunks without neighbors.

### 3. Palette Buffer Write

```
device.writeBuffer(
    chunk_palette_buf,
    offset = slot * MAX_PALETTE_SIZE * 4,
    data   = palette_material_ids[]
)
```

Writes the palette array of MaterialId values. Palette size varies per chunk (typically 1-16 entries, max 64K).

### 4. Index Buffer Write

```
device.writeBuffer(
    chunk_index_buf,
    offset = slot * MAX_INDEX_BUF_SIZE * 4,
    data   = bitpacked_palette_indices[]
)
```

Writes the per-voxel palette index buffer. Bit width auto-scales with palette size (1-16 bits per voxel).

### 5. Palette Metadata Write

```
device.writeBuffer(
    palette_meta,
    offset = slot * 4,
    data   = (palette_size as u16) | ((bits_per_entry as u8) << 16)
)
```

Writes `palette_size` and `bits_per_entry` so GPU shaders can unpack the index buffer correctly.

### 6. Coordinate Write

```
device.writeBuffer(
    chunk_coord,
    offset = slot * 16,
    data   = [chunk_coord.x, chunk_coord.y, chunk_coord.z, 0]  // vec4i, .w unused
)
```

### 7. Slot Table GPU Update

```
device.writeBuffer(
    chunk_slot_table_gpu,
    offset = slot_table_entry_offset(chunk_coord),
    data   = slot
)
```

Updates the GPU-resident flat lookup table so traversal shaders can resolve `world_coord -> slot_index`.

### 8. Residency and Version Stamp

```
device.writeBuffer(
    chunk_resident_flags,
    offset = slot * 4,
    data   = 1   // is_resident = 1
)

device.writeBuffer(
    chunk_version,
    offset = slot * 4,
    data   = 1   // initial version (0 = uninitialized, 1 = first upload)
)
```

### 9. Stale Summary Signal

```
// CPU sets the stale_summary bit for this slot
// This triggers the GPU compaction pass to enqueue the slot into summary_rebuild_queue
// CPU must NOT write directly to summary_rebuild_queue — queues are populated
// exclusively by the GPU compaction pass from stale bitsets (see edit-protocol)
set_stale_summary_bit(slot)
```

---

## Outputs

| Buffer | Access | Per-slot size | What's written |
|---|---|---|---|
| `chunk_occupancy_atlas[slot]` | Write | 32 KB (8192 x u32) | Full 64x64x64 bitpacked occupancy |
| `chunk_palette_buf[slot]` | Write | Variable (max 256 KB) | Palette MaterialId entries |
| `chunk_index_buf[slot]` | Write | Variable | Bitpacked per-voxel palette indices |
| `palette_meta[slot]` | Write | 4 B | `palette_size` (u16) + `bits_per_entry` (u8) |
| `chunk_coord[slot]` | Write | 16 B (vec4i) | World-space chunk coordinate |
| `chunk_slot_table_gpu` | Write | Entry size | coord -> slot mapping |
| `chunk_resident_flags[slot]` | Write | 4 B | `is_resident = 1` |
| `chunk_version[slot]` | Write | 4 B | Initial version stamp |
| `stale_summary` bit | Write | 1 bit | Flags slot for I-3 summary rebuild |

---

## Postconditions

| ID | Condition | Validates |
|---|---|---|
| POST-1 | `chunk_occupancy_atlas[slot]` matches the CPU-side `opaque_mask` exactly, byte-for-byte | Upload fidelity |
| POST-2 | `chunk_palette_buf[slot]` contains exactly the palette entries from the CPU-side `PaletteMaterials` | Palette fidelity |
| POST-3 | `chunk_index_buf[slot]` matches the CPU-side bitpacked index buffer exactly | Index fidelity |
| POST-4 | `palette_meta[slot].palette_size` and `.bits_per_entry` match the uploaded palette | Metadata consistency |
| POST-5 | `chunk_coord[slot]` matches the source `ChunkCoord` | Coordinate correctness |
| POST-6 | `chunk_resident_flags[slot] == 1` | Slot is marked resident |
| POST-7 | `chunk_version[slot] >= 1` | Version is initialized |
| POST-8 | `slot_table[chunk_coord] == slot` on both CPU and GPU | Directory consistency |
| POST-9 | `stale_summary` bit is set for this slot, ensuring I-3 will rebuild summaries | Summary rebuild queued |
| POST-10 | No other slot's data was modified by this upload | Slot isolation |

---

## Boundary Copy Sub-Step

After the primary upload, if neighbor chunks are already resident, a boundary copy pass updates the 1-voxel padding ring:

```
For each of the 6 faces (+X, -X, +Y, -Y, +Z, -Z):
  neighbor_coord = chunk_coord + face_normal
  if slot_table.contains(neighbor_coord):
    neighbor_slot = slot_table[neighbor_coord]
    // Copy this chunk's boundary row into neighbor's padding
    // Copy neighbor's boundary row into this chunk's padding
    copy_boundary(slot, neighbor_slot, face)
    // Mark neighbor as stale for summary rebuild
    set_stale_summary_bit(neighbor_slot)
```

The boundary copy ensures that face visibility computation (I-3, R-1) correctly handles chunk boundaries. Padding voxels at x=0 duplicate neighbor's x=62, etc. (see OCC-2, OCC-3 in [chunk-occupancy-atlas](../data/chunk-occupancy-atlas.md)).

---

## Transfer Method

All writes use `device.queue.writeBuffer()` for the initial implementation. This is a synchronous CPU -> GPU copy that is simple and correct.

Future optimization: use mapped staging buffers (`GPUBuffer` with `MAP_WRITE` usage) for double-buffered async upload. This would allow the CPU to prepare the next chunk's data while the previous chunk's data is being transferred.

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Roundtrip fidelity:** Upload a known occupancy pattern, readback via `mapAsync`, verify byte-for-byte match.
2. **Palette roundtrip:** Upload a palette with 5 materials, readback, verify all entries match.
3. **Index buffer roundtrip:** Upload a 4-bit-per-voxel index buffer, readback, verify all indices decode correctly.
4. **Coordinate roundtrip:** Upload a chunk at (-3, 7, 12), readback `chunk_coord[slot]`, verify match.
5. **Slot isolation:** Upload to slot 5, verify slots 4 and 6 are unchanged.

### Property tests (Rust, randomized)

6. **Random occupancy roundtrip:** Generate 100 random occupancy patterns, upload each, readback, verify all bits match.
7. **Random palette size:** Upload chunks with palette sizes from 1 to 256, verify `palette_meta` and `palette_buf` are consistent.
8. **Sequential uploads:** Upload N chunks to sequential slots, verify no cross-slot contamination.

### Integration tests

9. **I-1 -> I-2 pipeline:** Run voxelizer on a test mesh, upload result via I-2, readback and verify occupancy matches the voxelizer's CPU reference output.
10. **Residency flag:** After upload, verify `chunk_resident_flags[slot] == 1`. After eviction, verify `chunk_resident_flags[slot] == 0`.
11. **Boundary copy:** Upload two adjacent chunks, verify padding voxels in each match the neighbor's boundary.

### Cross-stage tests

12. **I-2 -> I-3:** After upload, verify I-3 summary rebuild produces correct `chunk_flags`, `occupancy_summary`, and `chunk_aabb` for the uploaded data.
13. **I-2 -> R-1:** After upload and I-3 rebuild, verify the greedy mesher produces valid geometry from the uploaded occupancy.
14. **Version gating:** Upload a chunk, start a rebuild, upload again (version increments). Verify the first rebuild's result is discarded by the swap pass due to version mismatch.

---

## See Also

- [pipeline-stages](../pipeline-stages.md) -- Stage I-2 buffer table and position in the ingest pipeline
- [chunk-contract](../chunk-contract.md) -- canonical chunk fields and residency contract
- [gpu-chunk-pool](../gpu-chunk-pool.md) -- slot allocation, atlas layout, slot lifecycle
- [edit-protocol](../edit-protocol.md) -- stale_summary signaling; why CPU must not write queues directly
- [chunk-occupancy-atlas](../data/chunk-occupancy-atlas.md) -- occupancy layout, padding invariants (OCC-1 through OCC-6)
- [I-1-voxelization](I-1-voxelization.md) -- produces the `CompactVoxel[]` input to I-2
- [I-3-summary-rebuild](I-3-summary-rebuild.md) -- consumes I-2 output; triggered by stale_summary bit
