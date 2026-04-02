# Test: Ingest Chain (I-1 → I-2 → I-3)

**Type:** spec
**Status:** current
**Date:** 2026-03-22

> Proves the ingest pipeline is logically consistent: voxelization output is valid pool input, upload preserves data, and summary derivation is correct.

---

## What This Tests

The ingest chain transforms external mesh data into GPU-resident chunk state:

```
Mesh triangles (I-1) → chunk occupancy bits (I-2) → derived summaries (I-3)
```

Each transition has a contract. This document defines the tests that prove those contracts hold.

---

## Chain Link 1: I-1 → I-2

**Claim:** The voxelizer's output (per-chunk occupancy bitmasks) is a valid input to the chunk pool upload.

### Preconditions (I-2 input contract)

| ID | What I-2 requires | How I-1 must satisfy it |
|---|---|---|
| L1-1 | Occupancy data is exactly 8192 u32 per chunk | I-1 produces `[u64; 4096]` which is `[u32; 8192]` — stored as 8192 u32 words (column-major). Must verify layout match. |
| L1-2 | Voxel at (x, y, z) maps to column `x*64+z`, bit `y` | I-1 must use the same addressing convention |
| L1-3 | Chunk coordinates are valid (non-negative, within world bounds) | I-1 assigns chunk coords from spatial partitioning |
| L1-4 | Palette entries reference valid MaterialIds in the material table | I-1 maps triangle materials to palette entries |

### Tests

```
T-L1-1: Addressing roundtrip
  For each (x, y, z) ∈ [0, 63]³:
    Write 1 via I-1's output format
    Read via I-2's input format
    Assert: same bit is set

T-L1-2: Layout size
  Assert: I-1 output for one chunk is exactly 32768 bytes

T-L1-3: Empty voxelization
  Voxelize a mesh that doesn't intersect any voxels in a chunk
  Assert: all 8192 words are 0
  Assert: I-2 upload succeeds
  Assert: I-3 produces is_empty=1

T-L1-4: Palette validity
  After voxelization, for each palette entry p:
    Assert: material_table[p] exists
    Assert: material_table[p] has valid albedo/emissive values
```

---

## Chain Link 2: I-2 → I-3

**Claim:** After upload, the GPU buffer contents exactly match the CPU-side data, and I-3 can derive correct summaries from them.

### Preconditions (I-3 input contract)

| ID | What I-3 requires | How I-2 must satisfy it |
|---|---|---|
| L2-1 | `chunk_occupancy_atlas[slot]` contains the uploaded occupancy | `writeBuffer` is synchronous from GPU queue perspective |
| L2-2 | `chunk_resident_flags[slot] == 1` | Pool manager sets this during I-2 |
| L2-3 | `chunk_coord[slot]` is set to the chunk's world coordinate | Pool manager writes this during I-2 |
| L2-4 | `chunk_version[slot]` has been incremented | Pool manager increments on upload |

### Tests

```
T-L2-1: Upload fidelity
  Write known occupancy pattern via I-2
  Readback via mapAsync
  Assert: every byte matches the input

T-L2-2: Resident flag set
  After I-2 upload to slot S:
    Assert: chunk_resident_flags[S] == 1

T-L2-3: Coord set
  After I-2 upload of chunk at world coord (cx, cy, cz):
    Assert: chunk_coord[S] == (cx, cy, cz, 0)

T-L2-4: Version increment
  Record version before upload: v0 = chunk_version[S]
  Perform I-2 upload
  Assert: chunk_version[S] > v0
```

---

## Chain Link 3: I-3 Postconditions

**Claim:** I-3's output is consistent with its input and satisfies the preconditions of all downstream stages.

### Tests

```
T-L3-1: Empty chunk propagation
  Upload an all-zero chunk via I-2
  Run I-3
  Assert: chunk_flags.is_empty == 1
  Assert: occupancy_summary all zeros
  Assert: chunk_aabb is degenerate (min > max or min == max)

T-L3-2: Full chunk propagation
  Upload an all-ones chunk via I-2
  Run I-3
  Assert: chunk_flags.is_empty == 0
  Assert: chunk_flags.is_fully_opaque == 1
  Assert: occupancy_summary all ones
  Assert: chunk_aabb == full chunk world extent

T-L3-3: Single voxel precision
  Upload a chunk with exactly one voxel at (17, 42, 31)
  Run I-3
  Assert: is_empty == 0
  Assert: exactly one bricklet bit set (bricklet (2, 5, 3))
  Assert: AABB min == chunk_world_origin + (17, 42, 31)
  Assert: AABB max == chunk_world_origin + (18, 43, 32)

T-L3-4: Emissive detection end-to-end
  Create a material with emissive > 0 in material_table
  Upload a chunk whose palette references that material
  Run I-3
  Assert: chunk_flags.has_emissive == 1

T-L3-5: Stale flag cleared
  Set chunk_flags.stale_summary = 1
  Run I-3
  Assert: chunk_flags.stale_summary == 0
```

---

## Full Chain Integration Test

```
T-FULL-1: Mesh → voxelize → upload → summarize → validate
  Input: procedural sphere mesh at known position
  Run I-1 (voxelization)
  Run I-2 (upload to pool slots)
  Run I-3 (summary rebuild)

  For each chunk:
    If occupied:
      Assert: is_empty == 0
      Assert: is_resident == 1
      Assert: AABB encloses all occupied voxels
      Assert: summary bits are correct for all 512 bricklets
    If not occupied:
      Assert: is_empty == 1
      Assert: occupancy_summary == all zeros

  Across all chunks:
    Assert: total occupied voxel count matches expected (sphere volume)
    Assert: no two chunks overlap in world space
    Assert: occupied voxels form a connected sphere shape (within voxelization tolerance)
```

---

## Consistency Properties (Hold for Any Valid Input)

```
P-1: For every slot S with is_resident == 1:
  chunk_flags.is_empty == (popcount(occupancy_atlas[S]) == 0)

P-2: For every slot S with is_resident == 1 and is_empty == 0:
  chunk_aabb.min < chunk_aabb.max (component-wise)
  Every occupied voxel is inside the AABB

P-3: For every slot S with is_resident == 1:
  occupancy_summary bit b == 1 ⟺ bricklet b has at least one occupied voxel

P-4: For every slot S with is_resident == 0:
  Derived data (flags, summary, aabb) is undefined — no consumer may read it
```

These properties are testable as assertions after any ingest operation and serve as the precondition for all render stages.
