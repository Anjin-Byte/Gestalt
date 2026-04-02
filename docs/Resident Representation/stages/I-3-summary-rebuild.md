# Stage I-3: Derived Summary Rebuild

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Stage type:** GPU compute
**Trigger:** After I-2 (chunk upload) or after edit kernels modify occupancy.

> Reads authoritative occupancy + palette. Writes derived summaries, flags, and AABBs. The bridge between raw voxel data and everything that consumes it.

---

## Purpose

Every consumer of chunk data (mesh builder, depth prepass, culling, traversal, cascades) needs summary information: Is this chunk empty? What's its bounding box? Does it have emissive materials? Computing these per-consumer would be redundant. I-3 computes them once and writes them to shared derived buffers.

---

## Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-1 | `chunk_occupancy_atlas[slot]` contains valid occupancy data | I-2 postcondition |
| PRE-2 | `chunk_palette[slot]` contains valid palette entries | I-2 postcondition |
| PRE-3 | `material_table` is populated with valid material entries | Scene init |
| PRE-4 | `chunk_resident_flags[slot] == 1` for all slots being rebuilt | Pool manager |
| PRE-5 | `chunk_coord[slot]` contains the correct world-space chunk coordinate | I-2 postcondition |

---

## Inputs

| Buffer | Access | Per-slot size | What's read |
|---|---|---|---|
| `chunk_occupancy_atlas` | Read | 32 KB | All 8192 u32 words (4096 columns × 2 words) |
| `chunk_palette[slot]` | Read | Variable | Palette entry MaterialIds |
| `material_table` | Read | 16 B × entry count | `emissive` field of each material |
| `chunk_coord[slot]` | Read | 16 B | World-space origin for AABB computation |

---

## Transformation

For each slot in the rebuild queue:

### 1. Occupancy Summary (bricklet grid)

Divide the 64³ chunk into 8×8×8 = 512 bricklets, each 8³ voxels.

For bricklet at (bx, by, bz) where bx, by, bz ∈ [0, 7]:

```
bricklet_occupied = false
for x in [bx*8 .. bx*8+7]:
  for z in [bz*8 .. bz*8+7]:
    column = occupancy_atlas[slot_offset + x*64 + z]  // as u64
    mask = column >> (by * 8) & 0xFF  // extract 8 Y-bits for this bricklet
    if mask != 0: bricklet_occupied = true; break
```

Write bit `bx * 64 + by * 8 + bz` in `occupancy_summary[slot]`.

**Optimized:** Each bricklet test is 8 columns × 1 byte extract. Total: 512 bricklets × 8 column reads = 4096 reads (same as the number of columns — each column is read once across all bricklets sharing its X-Z position).

### 2. Chunk Flags

Scan the occupancy atlas:
- `is_empty`: `popcount(all 8192 words) == 0`
- `is_fully_opaque`: all words in the inner region (x, z ∈ [1, 62]) are `0xFFFFFFFF`

Scan the palette against the material table:
- `has_emissive`: any `material_table[palette[i]].emissive > 0`

Set `is_resident` from `chunk_resident_flags[slot]`.

Clear stale bits: `stale_summary = 0` (this rebuild clears it).

### 3. Chunk AABB

Find the tight axis-aligned bounding box of occupied voxels:

```
min_x = 64, min_y = 64, min_z = 64
max_x = -1, max_y = -1, max_z = -1

for x in [0, 63]:
  for z in [0, 63]:
    column = occupancy_atlas[slot_offset + x*64 + z] as u64
    if column == 0: continue
    low_y  = ctz(column)      // first occupied Y
    high_y = 63 - clz(column) // last occupied Y
    min_x = min(min_x, x); max_x = max(max_x, x)
    min_z = min(min_z, z); max_z = max(max_z, z)
    min_y = min(min_y, low_y); max_y = max(max_y, high_y)
```

Convert to world space:
```
world_min = chunk_coord * 64 + (min_x, min_y, min_z)
world_max = chunk_coord * 64 + (max_x + 1, max_y + 1, max_z + 1)
```

Write `chunk_aabb[slot] = (world_min.xyz, 0, world_max.xyz, 0)` (two vec4f, .w unused).

---

## Outputs

| Buffer | Access | Per-slot size | What's written |
|---|---|---|---|
| `occupancy_summary[slot]` | Write | 64 B (16 × u32) | 512-bit bricklet grid |
| `chunk_flags[slot]` | Write | 4 B | Packed flag bits |
| `chunk_aabb[slot]` | Write | 32 B (2 × vec4f) | World-space tight AABB |

---

## Postconditions

| ID | Condition | Validates |
|---|---|---|
| POST-1 | `chunk_flags.is_empty == (popcount(occupancy) == 0)` | FLG-1 |
| POST-2 | `chunk_flags.is_fully_opaque == (all inner words == 0xFFFFFFFF)` | FLG-2 |
| POST-3 | `chunk_flags.has_emissive == (any palette entry is emissive)` | FLG-3 |
| POST-4 | `chunk_flags.stale_summary == 0` | Stale cleared after rebuild |
| POST-5 | `chunk_aabb.min <= chunk_aabb.max` component-wise (or chunk is empty → AABB is degenerate) | AABB validity |
| POST-6 | Every occupied voxel is inside the AABB | AABB tightness |
| POST-7 | `occupancy_summary` bit is 1 if and only if the corresponding 8³ bricklet has at least one occupied voxel | Summary correctness |
| POST-8 | For empty chunks, `occupancy_summary` is all zeros | Empty consistency |

---

## Dispatch

```
workgroup_size: (64, 1, 1)  — one thread per column (4096 columns / 64 = 64 workgroups per slot)
dispatch: (64, 1, 1) per chunk in rebuild queue
```

Each thread handles one column: reads the u64, contributes to shared bricklet bits, min/max AABB atomics.

Alternatively: one workgroup per slot, 256 threads, each handles 16 columns. Shared memory for bricklet accumulation and AABB reduction.

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Empty chunk:** All-zero occupancy → `is_empty=1`, `is_fully_opaque=0`, summary all zeros, AABB degenerate.
2. **Full chunk:** All-ones occupancy → `is_empty=0`, `is_fully_opaque=1`, summary all ones, AABB = full chunk extent.
3. **Single voxel:** One occupied voxel at known (x, y, z) → only one bricklet bit set, AABB = single voxel extent, `is_empty=0`.
4. **Emissive detection:** Palette with one emissive material → `has_emissive=1`. Non-emissive palette → `has_emissive=0`.

### Property tests (Rust, randomized)

5. **AABB containment:** For 1000 random occupancy patterns, verify every occupied voxel is inside the computed AABB.
6. **AABB tightness:** Verify at least one occupied voxel touches each face of the AABB.
7. **Summary completeness:** For each bricklet with `summary_bit == 1`, verify at least one voxel in that 8³ region is occupied. For `summary_bit == 0`, verify all voxels in that region are unoccupied.
8. **Flag consistency:** Verify `is_empty` matches full popcount. Verify `is_fully_opaque` matches inner-region all-ones check.

### GPU validation

9. **CPU ↔ GPU agreement:** Run I-3 on GPU, readback results, compare against CPU reference implementation for the same input.
10. **Idempotency:** Running I-3 twice on the same input produces identical output.

### Cross-stage tests

11. **I-3 → R-1:** Chunks with `is_empty=1` are never in the mesh rebuild queue.
12. **I-3 → R-4:** Chunks with `is_empty=1` are never in the cull input.
13. **I-3 → R-6:** Chunks with `is_empty=1` are skipped during DDA traversal.
