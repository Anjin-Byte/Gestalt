# Chunk Occupancy Atlas

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Authoritative data — never derived, written by producers only.

> The binary occupancy of every voxel in a chunk. The single source of truth for world state.

---

## Identity

- **Buffer name:** `chunk_occupancy_atlas`
- **WGSL type:** `array<u32>` (accessed as `array<u32, 2048>` per slot via offset)
- **GPU usage:** `STORAGE | COPY_DST`
- **Binding:** `@group(0) @binding(0)` in traversal, summary, and mesh shaders

---

## Layout

One slot occupies **32,768 bytes** (8,192 × `u32`).

This is 64³ = 262,144 voxels at 1 bit each = 262,144 bits = 8,192 u32 words.

The chunk is 64×64×64 voxels. Occupancy is stored as **column-major bitpacked u64**:

```
For voxel at (x, y, z) within the chunk:
  column_index = x * 64 + z          // 4096 columns (64 × 64 XZ plane)
  bit_index    = y                    // 0..63 within the column
  word_offset  = column_index * 2     // each column is 2 × u32 (= 1 × u64)
  u32_index    = word_offset + (y >> 5)  // which u32 within the pair
  bit_within   = y & 31                   // which bit within that u32

  is_occupied = (atlas[slot_offset + u32_index] >> bit_within) & 1
```

Where `slot_offset = slot_index * 8192`.

### Why column-major u64?

The Y axis is the gravity axis. Traversal rays typically march along Y (vertical) or XZ (horizontal floor scan). Packing Y into a single u64 per column enables:

- **Vertical skip:** `ctz(column >> current_y)` finds the next occupied voxel above in one instruction
- **Empty column skip:** `column == 0` skips the entire 64-voxel height
- **Boundary test:** `column & 1` and `column & (1 << 63)` test top/bottom in one op

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| OCC-1 | Every bit at (x, y, z) where x, y, z ∈ [0, 63] maps to exactly one column and bit position via the formula above | Layout definition |
| OCC-2 | The 1-voxel padding ring (x=0, x=63, z=0, z=63) duplicates neighbor chunk boundary data | Boundary copy pass (I-2) |
| OCC-3 | Inner voxels (x, z ∈ [1, 62]) are authoritative; padding voxels are derived copies | Edit kernels write inner only; boundary copy writes padding |
| OCC-4 | `chunk_version[slot]` increments monotonically on every write to this slot's occupancy | Edit kernel postcondition |
| OCC-5 | A slot with `chunk_resident_flags[slot] == 0` has undefined occupancy content | Pool lifecycle |
| OCC-6 | Total atlas size = `MAX_SLOTS * 8192 * 4` bytes | Buffer creation |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| Each u32 word | `0x00000000 .. 0xFFFFFFFF` | Any bit pattern is valid — represents occupancy |
| `slot_index` | `0 .. MAX_SLOTS-1` | Out-of-range = buffer overrun |
| `column_index` | `0 .. 4095` | `x * 64 + z` where x, z ∈ [0, 63] |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Voxelizer (I-1) | Mesh load / procedural gen | Full 64³ occupancy for new chunks |
| Edit kernels | Runtime voxel edits | Individual bits within inner region (x, z ∈ [1, 62]) |
| Boundary copy (I-2 sub-step) | After edit or load of neighbor | Padding ring from neighbor's boundary |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Summary rebuild | I-3 | Full slot scan → derive `occupancy_summary`, `chunk_flags`, `aabb` |
| Greedy mesher | R-1 | Full slot scan → derive `vertex_pool`, `index_pool` |
| DDA traversal | R-6 | Random access per-column during ray march |
| Debug viz | R-9 | Full slot scan for occupancy heatmap |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Addressing correctness:** For all (x, y, z) in [0, 63]³, verify the bit read from the computed offset matches the bit written.
2. **Column isolation:** Writing to column (x, z) does not modify any other column.
3. **Boundary padding:** After boundary copy, padding voxels at x=0 match neighbor's x=62, etc.
4. **Empty chunk:** All zeros → every voxel reads as unoccupied.
5. **Full chunk:** All ones → every voxel reads as occupied.

### Property tests (Rust, randomized)

6. **Roundtrip:** Generate random occupancy, write to atlas, read back all 64³ voxels — every bit matches.
7. **Column skip:** For a random column, verify `ctz(column >> y)` correctly finds the next occupied Y above y.
8. **Slot isolation:** Writing to slot N does not affect slot N+1 or N-1.

### GPU validation (WGSL compute)

9. **Readback test:** Write known pattern from CPU, dispatch compute shader that reads every voxel, verify via readback buffer.
10. **Column DDA test:** Dispatch traversal on a single known column, verify hit/miss sequence matches CPU reference.
