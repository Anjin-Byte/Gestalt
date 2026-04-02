# Test: Material Consistency (PAL / IDX / MAT)

**Type:** spec
**Status:** current
**Date:** 2026-03-22

> Proves the material pipeline is consistent end-to-end: every voxel's palette index is in range, every palette entry maps to a valid material, bit widths are correct, reserved slots are initialized, and property changes propagate to all affected chunks.

---

## What This Tests

Three buffers form the material lookup chain for every occupied voxel:

```
chunk_index_buf  (per-voxel palette index, bitpacked)
       |
       v
chunk_palette    (per-chunk list of unique MaterialIds)
       |
       v
material_table   (global MaterialId -> PBR properties)
```

This document defines the tests that prove the data within and across these three buffers is self-consistent.

---

## Structures Under Test

| Buffer | Invariants | Spec |
|---|---|---|
| `chunk_palette_buf` | PAL-1 through PAL-6 | `data/chunk-palette.md` |
| `chunk_index_buf` + `palette_meta` | IDX-1 through IDX-5 | `data/chunk-index-buf.md` |
| `material_table` | MAT-1 through MAT-7 | `data/material-table.md` |

---

## 1. Every Palette Entry Maps to a Valid Material (PAL-1)

**Claim:** Every entry in a chunk's palette is a MaterialId that exists and is valid within the material table.

```
T-PAL-VALID-1: Palette -> material_table lookup
  For every resident slot S:
    Read palette_size from palette_meta[S]
    For each entry i in [0, palette_size-1]:
      Read material_id = palette[S][i]
      Assert: material_id in [1, 4095]  (PAL-1: valid range, PAL-5: not MATERIAL_EMPTY)
      Assert: material_table[material_id] has valid property values (MAT-1 through MAT-4)

T-PAL-VALID-2: No out-of-range MaterialIds
  For every resident slot S:
    For each palette entry:
      Assert: entry < MAX_MATERIALS (4096)

T-PAL-VALID-3: After voxelizer output
  Run voxelizer on a mesh with known materials M1, M2, M3
  For each produced chunk:
    Assert: palette contains only a subset of {M1, M2, M3}
    Assert: every palette entry maps to the correct material properties
```

---

## 2. MATERIAL_EMPTY Never in a Palette (PAL-5)

**Claim:** MaterialId 0 (MATERIAL_EMPTY) is never stored as a palette entry. Empty voxels are represented by the occupancy atlas (bit == 0), not by a material.

```
T-PAL-EMPTY-1: Fresh chunk palette
  After voxelization of any mesh:
    For each palette entry in each chunk:
      Assert: entry != 0x0000

T-PAL-EMPTY-2: After edit adding new material
  Edit a voxel to use a new material
  Assert: new palette entry != 0x0000

T-PAL-EMPTY-3: After edit removing last voxel of a material
  Remove all voxels of material M from a chunk (set them to empty)
  Assert: 0x0000 is not added to the palette
  (Material M may remain in the palette as unused, or be compacted out —
   but MATERIAL_EMPTY must never appear.)

T-PAL-EMPTY-4: Exhaustive scan
  For every resident slot S:
    For each entry i in [0, palette_size-1]:
      Assert: palette[S][i] != 0x0000
```

---

## 3. No Duplicate Palette Entries (PAL-2)

**Claim:** Each chunk's palette contains no repeated MaterialIds.

```
T-PAL-DEDUP-1: After voxelization
  For each chunk produced by voxelizer:
    Collect all palette entries into a set
    Assert: set.len() == palette_size (no duplicates)

T-PAL-DEDUP-2: After edit introducing existing material
  Chunk has palette [M1, M2, M3]
  Edit a voxel from M1 to M2
  Assert: palette still has no duplicates
  Assert: palette_size unchanged or decreased (M1 may be removed if no longer used)

T-PAL-DEDUP-3: Property test
  For 500 random chunks:
    Assert: palette entries form a set (all unique)
```

---

## 4. Every Voxel's palette_idx Is in Range (IDX-2)

**Claim:** For every occupied voxel, its bitpacked palette index is strictly less than the chunk's palette_size.

```
T-IDX-RANGE-1: Exhaustive scan per chunk
  For every resident slot S:
    Read bpe and palette_size from palette_meta[S]
    For each voxel (x, y, z) where occupancy bit == 1:
      Read palette_idx via bitpacking formula
      Assert: palette_idx < palette_size

T-IDX-RANGE-2: Maximum palette index
  Create a chunk with exactly 256 materials (maximum palette)
  Assert: bpe == 8
  For each occupied voxel:
    Assert: palette_idx in [0, 255]

T-IDX-RANGE-3: Minimum palette (single material)
  Create a chunk with exactly 1 material
  Assert: bpe == 1
  For each occupied voxel:
    Assert: palette_idx == 0

T-IDX-RANGE-4: After palette resize
  Start with palette_size = 4 (bpe = 2)
  Edit to introduce a 5th material (triggers resize to bpe = 4)
  For each occupied voxel:
    Assert: palette_idx < 5 (new palette_size)
    Assert: palette_idx values are preserved from before resize
```

---

## 5. bits_per_entry Matches palette_size (IDX-1)

**Claim:** The bit width stored in `palette_meta` is the smallest valid width that can represent all palette entries.

```
T-BPE-MATCH-1: All valid transitions
  | palette_size | Expected bpe |
  |---|---|
  | 1-2          | 1            |
  | 3-4          | 2            |
  | 5-16         | 4            |
  | 17-256       | 8            |

  For each (palette_size, expected_bpe):
    Create a chunk with exactly that many materials
    Assert: palette_meta bits 16-23 == expected_bpe

T-BPE-MATCH-2: Boundary cases
  Test palette_size = 2 (bpe=1), 3 (bpe=2), 5 (bpe=4), 17 (bpe=8)
  Assert: each produces the correct bpe

T-BPE-MATCH-3: Invalid bpe rejection
  Attempt to set palette_meta with bpe = 3 (not in {1,2,4,8})
  Assert: validation rejects or corrects

T-BPE-MATCH-4: bpe consistency after edit
  Start with palette_size = 16 (bpe = 4)
  Add one material (palette_size = 17)
  Assert: bpe transitions to 8
  Remove materials down to palette_size = 4
  Assert: bpe may remain at 8 or compact to 2 (implementation-dependent)
    If compaction occurs, verify all voxel indices are re-encoded correctly
```

---

## 6. Reserved Bits Are Zero (IDX-5, FLG-6)

**Claim:** Reserved fields in `palette_meta` and related structures are always zero.

```
T-RSVD-META-1: palette_meta reserved bits
  For every resident slot S:
    Assert: (palette_meta[S] >> 24) & 0xFF == 0

T-RSVD-META-2: After every write
  After voxelization, edit, or palette resize:
    Assert: (palette_meta[S] >> 24) & 0xFF == 0

T-RSVD-META-3: Sweep all slots
  For every slot in [0, MAX_SLOTS-1]:
    If resident:
      Assert: (palette_meta[S] >> 24) & 0xFF == 0
```

---

## 7. Material Table Reserved Entries (MAT-5, MAT-6)

**Claim:** `material_table[0]` (MATERIAL_EMPTY) is all-zero and `material_table[1]` (MATERIAL_DEFAULT) has the specified default values.

```
T-MAT-RESERVED-1: MATERIAL_EMPTY
  Assert: material_table[0].albedo_rg == 0
  Assert: material_table[0].albedo_b_roughness == 0
  Assert: material_table[0].emissive_rg == 0
  Assert: material_table[0].emissive_b_opacity == 0

T-MAT-RESERVED-2: MATERIAL_DEFAULT
  Unpack material_table[1]:
    Assert: albedo == (0.5, 0.5, 0.5) within f16 precision
    Assert: roughness == 0.5 within f16 precision
    Assert: opacity == 1.0 within f16 precision
    Assert: emissive == (0.0, 0.0, 0.0)

T-MAT-RESERVED-3: Reserved entries survive runtime changes
  Register 100 custom materials at IDs 2-101
  Assert: material_table[0] is still all-zero
  Assert: material_table[1] still matches MATERIAL_DEFAULT
```

---

## 8. Material Property Ranges (MAT-1 through MAT-4)

**Claim:** All material properties are within their valid ranges.

```
T-MAT-RANGE-1: Albedo clamped to [0, 1]
  Attempt to register a material with albedo.r = 1.5
  Assert: validation rejects or clamps to 1.0

T-MAT-RANGE-2: Roughness clamped to [0, 1]
  Attempt to register roughness = -0.1
  Assert: validation rejects or clamps to 0.0

T-MAT-RANGE-3: Opacity clamped to [0, 1]
  Attempt to register opacity = 2.0
  Assert: validation rejects or clamps to 1.0

T-MAT-RANGE-4: Emissive non-negative
  Attempt to register emissive.r = -1.0
  Assert: validation rejects or clamps to 0.0

T-MAT-RANGE-5: Valid material roundtrip (property test)
  For 500 random materials with valid property values:
    Pack to MaterialEntry (4 x u32)
    Unpack via f16 extraction
    Assert: all values match within f16 precision (relative error < 0.001 for values > 0.01)

T-MAT-RANGE-6: Exhaustive table scan
  For every registered material M in [0, 4095]:
    Unpack properties
    Assert: albedo channels in [0.0, 1.0]
    Assert: roughness in [0.0, 1.0]
    Assert: opacity in [0.0, 1.0]
    Assert: emissive channels in [0.0, 65504.0]
```

---

## 9. Material Table Size (MAT-7)

**Claim:** The material table buffer is exactly 65,536 bytes (4096 entries x 16 bytes).

```
T-MAT-SIZE-1: Buffer size
  Assert: material_table buffer byte length == 65536

T-MAT-SIZE-2: Entry count
  Assert: material_table can store exactly 4096 entries
  Assert: accessing index 4095 is valid
  Assert: accessing index 4096 is out-of-bounds
```

---

## 10. Material Property Change Invalidates Affected Chunks

**Claim:** When a material's properties are updated at runtime, all chunks whose palettes reference that material have their stale bits set.

```
T-MAT-INVAL-1: Single material change
  Register material M at ID 42 with albedo (1, 0, 0)
  Create 3 chunks, 2 of which reference material 42 in their palettes
  Update material 42 to albedo (0, 1, 0)
  Assert: the 2 chunks referencing material 42 have stale_summary == 1
  Assert: the chunk NOT referencing material 42 also has stale_summary == 1
    (conservative invalidation: all resident slots are marked stale per spec)

T-MAT-INVAL-2: Emissive toggle
  Register material M with emissive == 0
  Create a chunk referencing M
  Run I-3
  Assert: chunk_flags.has_emissive == 0
  Update M to have emissive > 0
  Run I-3 (after stale propagation)
  Assert: chunk_flags.has_emissive == 1

T-MAT-INVAL-3: Version tracking
  Record material_table_version before change: v0
  Update any material property
  Assert: material_table_version > v0

T-MAT-INVAL-4: No stale bits if nothing changed
  Read material M's properties
  Write back the same properties (no-op change)
  Assert: implementation either skips invalidation or conservatively sets stale
    (both are valid; this test documents which behavior the implementation chooses)
```

---

## 11. End-to-End Material Lookup Chain

**Claim:** For any occupied voxel, the full chain `index_buf -> palette -> material_table` produces valid PBR properties.

```
T-E2E-LOOKUP-1: Single voxel lookup
  Create a chunk with one material (M = 7, albedo = (0.8, 0.2, 0.1))
  Set one voxel as occupied
  Read palette_idx from index_buf for that voxel
  Assert: palette_idx == 0 (single-material palette)
  Read material_id from palette[0]
  Assert: material_id == 7
  Read properties from material_table[7]
  Assert: albedo == (0.8, 0.2, 0.1) within f16 precision

T-E2E-LOOKUP-2: Multi-material chunk
  Create a chunk with 5 materials
  For each occupied voxel:
    Read palette_idx from index_buf
    Assert: palette_idx < 5
    Read material_id from palette[palette_idx]
    Assert: material_id in [1, 4095]
    Read properties from material_table[material_id]
    Assert: all property ranges valid

T-E2E-LOOKUP-3: Maximum materials (256)
  Create a chunk with exactly 256 unique materials
  Assert: bpe == 8
  For 1000 randomly sampled occupied voxels:
    Trace the full lookup chain
    Assert: every step produces valid data

T-E2E-LOOKUP-4: After edit changes material
  Chunk has voxel V with material M1
  Edit voxel V to use material M2
  Read palette_idx for V from index_buf
  Assert: palette[palette_idx] == M2
  Assert: material_table[M2] has correct properties
```

---

## 12. Fast Emissive Test Consistency

**Claim:** The `mat_is_emissive` fast test in WGSL produces the same result as a full unpack-and-compare.

```
T-EMISSIVE-FAST-1: Zero emissive
  Pack a material with emissive = (0, 0, 0)
  Assert: (emissive_rg == 0) && ((emissive_b_opacity & 0xFFFF) == 0)
  Assert: mat_is_emissive == false

T-EMISSIVE-FAST-2: Non-zero emissive R
  Pack a material with emissive = (1.0, 0, 0)
  Assert: emissive_rg != 0
  Assert: mat_is_emissive == true

T-EMISSIVE-FAST-3: Non-zero emissive B only
  Pack a material with emissive = (0, 0, 0.5)
  Assert: (emissive_b_opacity & 0xFFFF) != 0
  Assert: mat_is_emissive == true

T-EMISSIVE-FAST-4: Property test
  For 1000 random materials:
    Pack, run fast test, unpack and compare
    Assert: fast test result matches (emissive.r > 0 || emissive.g > 0 || emissive.b > 0)
```

---

## Consistency Properties (Hold for Any Valid Material State)

```
P-MAT-1: For every resident slot S and every occupied voxel (x,y,z) in slot S:
  palette_idx(x,y,z) < palette_size(S)

P-MAT-2: For every resident slot S and every entry i in [0, palette_size-1]:
  palette[S][i] in [1, MAX_MATERIALS-1]
  palette[S][i] != 0 (not MATERIAL_EMPTY)

P-MAT-3: For every resident slot S:
  palette entries are unique (no duplicates)

P-MAT-4: For every material_id M in any resident palette:
  material_table[M] has albedo in [0,1]^3, roughness in [0,1], opacity in [0,1], emissive >= 0

P-MAT-5: material_table[0] is all-zero
  material_table[1] matches MATERIAL_DEFAULT

P-MAT-6: For every resident slot S:
  bits_per_entry(S) in {1, 2, 4, 8}
  palette_meta(S) bits 24-31 == 0
```

These properties are testable as assertions after any material write, palette change, or voxelizer output and serve as preconditions for fragment shading, emissive scanning, and cascade builds.
