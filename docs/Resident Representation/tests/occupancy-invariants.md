# Test: Occupancy Invariants (OCC / FLG / SUM)

**Type:** spec
**Status:** current
**Date:** 2026-03-22

> Proves that occupancy data is internally consistent across the atlas, flags, and summary buffers. Every voxel must be addressable, padding must agree with neighbors, and coarse summaries must faithfully reflect fine-grained occupancy.

---

## What This Tests

Three buffers jointly describe occupancy state for every resident chunk:

```
chunk_occupancy_atlas  (authoritative, per-voxel bits)
        |
        +--> chunk_flags          (derived, per-slot summary bits)
        +--> occupancy_summary    (derived, per-bricklet summary bits)
```

This document defines the tests that prove the data within and across these three buffers is self-consistent.

---

## Structures Under Test

| Buffer | Invariants | Spec |
|---|---|---|
| `chunk_occupancy_atlas` | OCC-1 through OCC-6 | `data/chunk-occupancy-atlas.md` |
| `chunk_flags` | FLG-1, FLG-2, FLG-6 | `data/chunk-flags.md` |
| `occupancy_summary` | SUM-1 through SUM-3 | `data/occupancy-summary.md` |

---

## 1. Column Addressing Roundtrip (OCC-1)

**Claim:** Every voxel at (x, y, z) where x, y, z in [0, 63] maps to exactly one column and bit position, and a write/read roundtrip through the atlas formula is lossless.

```
T-OCC-RT-1: Exhaustive addressing roundtrip
  For each (x, y, z) in [0, 63]^3 (262,144 voxels):
    Compute column_index = x * 64 + z
    Compute u32_index    = column_index * 2 + (y >> 5)
    Compute bit_within   = y & 31
    Set the bit at (slot_offset + u32_index, bit_within)
    Read back via the same formula
    Assert: bit is set
    Clear the bit
    Assert: bit is cleared

T-OCC-RT-2: No aliasing
  For each pair of distinct voxels (x1,y1,z1) != (x2,y2,z2):
    Set only (x1,y1,z1) in a zeroed atlas
    Read (x2,y2,z2)
    Assert: reads as 0
  (Implemented as: set one voxel, scan all 262,144, assert popcount == 1.
   Repeat for a representative sample of 1,000 random positions.)

T-OCC-RT-3: Column isolation
  Write all bits in column (x, z)
  For every other column (x', z') != (x, z):
    Assert: column reads as 0
  (Repeat for 100 random columns.)
```

---

## 2. Padding and Boundary Consistency (OCC-2, OCC-3)

**Claim:** The 1-voxel padding ring duplicates neighbor boundary data, and inner voxels are never overwritten by the boundary copy pass.

```
T-OCC-PAD-1: Padding matches neighbor boundary
  Create two adjacent chunks A (at coord (0,0,0)) and B (at coord (1,0,0))
  Fill chunk B's column at x=1 (inner boundary) with a known pattern P
  Run boundary copy pass
  Assert: chunk A's column at x=63 (padding) matches P

T-OCC-PAD-2: All six faces
  For each face direction (+X, -X, +Z, -Z, +Y, -Y):
    Create a chunk and its neighbor on that face
    Fill the neighbor's inner boundary column/row with pattern P
    Run boundary copy pass
    Assert: the chunk's padding on that face matches P

T-OCC-PAD-3: Inner region preservation
  Fill chunk with known inner pattern (x, z in [1, 62])
  Run boundary copy pass with arbitrary neighbor data
  Assert: all inner voxels (x, z in [1, 62]) are unchanged

T-OCC-PAD-4: Corner and edge padding
  Create a chunk with all 26 neighbors populated
  Run boundary copy pass
  Assert: padding corners (e.g., (0,y,0), (63,y,63)) contain the correct
          neighbor data from the diagonally adjacent chunk
```

---

## 3. Empty and Full Detection (FLG-1, FLG-2)

**Claim:** `is_empty` and `is_fully_opaque` flags exactly match the occupancy popcount.

```
T-FLG-EMPTY-1: All-zero atlas
  Write all-zero occupancy for a slot
  Run I-3 (summary rebuild)
  Assert: chunk_flags.is_empty == 1
  Assert: chunk_flags.is_fully_opaque == 0

T-FLG-EMPTY-2: All-ones atlas
  Write all-ones occupancy for a slot (all 8192 u32 = 0xFFFFFFFF)
  Run I-3
  Assert: chunk_flags.is_empty == 0
  Assert: chunk_flags.is_fully_opaque == 1

T-FLG-EMPTY-3: Single voxel
  Write exactly one voxel at (31, 31, 31)
  Run I-3
  Assert: chunk_flags.is_empty == 0
  Assert: chunk_flags.is_fully_opaque == 0

T-FLG-EMPTY-4: Popcount consistency (property test)
  For 1,000 random occupancy patterns:
    Write pattern to slot
    Run I-3
    CPU-side: compute popcount of all 8192 u32 words
    Assert: is_empty == (popcount == 0)
    Assert: is_fully_opaque == (all inner-region words == 0xFFFFFFFF)

T-FLG-EMPTY-5: One bit shy of full
  Set all 8192 u32 to 0xFFFFFFFF, then clear one arbitrary bit
  Run I-3
  Assert: is_fully_opaque == 0
  Assert: is_empty == 0
```

---

## 4. Reserved Bits (FLG-6)

**Claim:** Bits 8-31 of `chunk_flags` are always zero after any flag computation.

```
T-FLG-RSVD-1: After fresh summary rebuild
  Run I-3 for a populated slot
  Assert: (chunk_flags[slot] & 0xFFFFFF00) == 0

T-FLG-RSVD-2: After edit + stale propagation
  Edit a voxel in a clean slot (triggers stale bits)
  Assert: (chunk_flags[slot] & 0xFFFFFF00) == 0

T-FLG-RSVD-3: Sweep all slots
  For every resident slot S:
    Assert: (chunk_flags[S] & 0xFFFFFF00) == 0
```

---

## 5. Bricklet Summary Completeness (SUM-1, SUM-2, SUM-3)

**Claim:** Each bricklet bit in `occupancy_summary` is 1 if and only if at least one voxel in that bricklet's 8x8x8 region is occupied.

```
T-SUM-COMP-1: Single voxel at each bricklet
  For each bricklet (bx, by, bz) in [0, 7]^3:
    Clear the atlas
    Set one voxel at (bx*8 + 4, by*8 + 4, bz*8 + 4)
    Run I-3
    Assert: summary bit for (bx, by, bz) == 1
    Assert: all other 511 summary bits == 0

T-SUM-COMP-2: Empty chunk
  Write all-zero occupancy
  Run I-3
  Assert: all 512 summary bits == 0
  Assert: chunk_flags.is_empty == 1 (cross-check with FLG-1)

T-SUM-COMP-3: Full chunk
  Write all-ones occupancy
  Run I-3
  Assert: all 512 summary bits == 1

T-SUM-COMP-4: Bricklet boundary voxel
  For each bricklet (bx, by, bz):
    Set only the corner voxel (bx*8, by*8, bz*8) — the first voxel in the bricklet
    Run I-3
    Assert: summary bit for (bx, by, bz) == 1
  Repeat with the last voxel (bx*8+7, by*8+7, bz*8+7)
    Assert: same result

T-SUM-COMP-5: Bricklet isolation
  Fill exactly one bricklet (3, 5, 2) completely (all 512 voxels inside it)
  Run I-3
  Assert: summary bit for (3, 5, 2) == 1
  Assert: all other 511 summary bits == 0

T-SUM-COMP-6: Roundtrip property test
  For 500 random occupancy patterns:
    Write pattern to atlas
    Run I-3
    For each bricklet (bx, by, bz):
      CPU-side: OR-reduce all voxels in [bx*8..bx*8+7] x [by*8..by*8+7] x [bz*8..bz*8+7]
      Assert: summary bit == (OR-reduction != 0)
```

---

## 6. AABB Containment (Cross-Structure)

**Claim:** The chunk AABB (derived by I-3) encloses every occupied voxel.

```
T-AABB-1: Single voxel AABB
  Set one voxel at (17, 42, 31) in a chunk at coord (cx, cy, cz)
  Run I-3
  Let world_origin = (cx, cy, cz) * 62 * voxel_size
  Assert: aabb.min == world_origin + (17, 42, 31) * voxel_size
  Assert: aabb.max == world_origin + (18, 43, 32) * voxel_size

T-AABB-2: All occupied voxels inside AABB
  For 200 random occupancy patterns:
    Write pattern, run I-3
    For each occupied voxel (x, y, z):
      Compute world_pos = chunk_world_origin + (x, y, z) * voxel_size
      Assert: aabb.min <= world_pos (component-wise)
      Assert: aabb.max >= world_pos + voxel_size (component-wise)

T-AABB-3: Empty chunk AABB
  Write all-zero occupancy
  Run I-3
  Assert: aabb is degenerate (min >= max or both zero)

T-AABB-4: Full chunk AABB
  Write all-ones occupancy
  Run I-3
  Assert: aabb.min == chunk_world_origin
  Assert: aabb.max == chunk_world_origin + (64, 64, 64) * voxel_size

T-AABB-5: Tight fit
  Set voxels at exactly (10, 20, 30) and (50, 40, 60)
  Run I-3
  Assert: aabb.min == chunk_world_origin + (10, 20, 30) * voxel_size
  Assert: aabb.max == chunk_world_origin + (51, 41, 61) * voxel_size
```

---

## 7. Cross-Structure Consistency (FLG-1 vs SUM-2)

**Claim:** `chunk_flags.is_empty` and `occupancy_summary` always agree.

```
T-CROSS-1: is_empty implies summary all-zero
  For every resident slot S:
    If chunk_flags[S].is_empty == 1:
      Assert: all 16 u32 of occupancy_summary[S] are 0

T-CROSS-2: summary any-nonzero implies not empty
  For every resident slot S:
    If any word in occupancy_summary[S] is nonzero:
      Assert: chunk_flags[S].is_empty == 0

T-CROSS-3: Random pattern cross-check
  For 500 random occupancy patterns:
    Write pattern, run I-3
    Let cpu_empty = (popcount of all atlas words == 0)
    Let cpu_summary_empty = (all 512 summary bits == 0)
    Assert: cpu_empty == cpu_summary_empty
    Assert: chunk_flags.is_empty == cpu_empty
```

---

## Consistency Properties (Hold for Any Valid Input)

```
P-OCC-1: For every slot S with is_resident == 1:
  chunk_flags.is_empty == (popcount(occupancy_atlas[S]) == 0)

P-OCC-2: For every slot S with is_resident == 1:
  occupancy_summary bit (bx,by,bz) == 1
    iff at least one voxel in that bricklet is occupied in occupancy_atlas[S]

P-OCC-3: For every slot S with is_resident == 1 and is_empty == 0:
  chunk_aabb contains every occupied voxel's world position

P-OCC-4: For every slot S with is_resident == 1:
  chunk_flags bits 8-31 == 0
```

These properties are testable as assertions after any occupancy write or summary rebuild and serve as preconditions for traversal, meshing, and culling stages.
