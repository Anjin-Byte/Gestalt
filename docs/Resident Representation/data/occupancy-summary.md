# Occupancy Summary

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Derived (produced by I-3 summary rebuild pass).

> Coarse bricklet occupancy grid: one bit per 8x8x8 region of the 64x64x64 chunk. Used by traversal DDA for empty-space skip at bricklet granularity (Level 0.5).

---

## Identity

- **Buffer name:** `occupancy_summary`
- **WGSL type:** `array<u32>` (accessed as 16 u32 per slot via offset)
- **GPU usage:** `STORAGE`
- **Binding:** `@group(0)` in traversal and summary shaders

---

## Layout

One slot occupies **64 bytes** (16 x `u32` = 512 bits).

The chunk's 64x64x64 voxel volume is divided into an 8x8x8 grid of bricklets, where each bricklet covers an 8x8x8 region of voxels. One bit per bricklet: set if at least one voxel in that bricklet is occupied.

```
For bricklet at (bx, by, bz) where bx, by, bz in [0, 7]:
  bit_index  = bx * 64 + by * 8 + bz    // 512 possible bricklets
  word_index = bit_index >> 5             // bit_index / 32
  bit_within = bit_index & 31             // bit_index % 32

  is_occupied = (occupancy_summary[slot_offset + word_index] >> bit_within) & 1
```

Where `slot_offset = slot_index * 16`.

### Bricklet-to-Voxel Mapping

Bricklet `(bx, by, bz)` covers voxels:
```
x in [bx * 8, bx * 8 + 7]
y in [by * 8, by * 8 + 7]
z in [bz * 8, bz * 8 + 7]
```

The summary is computed by OR-reducing all occupancy bits within each bricklet region of the `chunk_occupancy_atlas`.

### Traversal Use (Level 0.5)

In the three-level DDA, bricklet testing sits between chunk DDA (Level 0) and voxel DDA (Level 1):

```
Level 0:   Chunk DDA    — skip empty/non-resident chunks
Level 0.5: Bricklet DDA — skip empty 8x8x8 regions within a non-empty chunk
Level 1:   Voxel DDA    — per-voxel bit test in opaque_mask
```

The bricklet level is optional and only worth adding if profiling shows the voxel DDA inner loop spending significant time in empty space within non-empty chunks.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| SUM-1 | A bricklet bit is 1 if and only if at least one voxel in that bricklet's 8x8x8 region is occupied in `chunk_occupancy_atlas` | Summary rebuild pass (I-3) |
| SUM-2 | All 512 bits are zero if and only if the chunk is entirely empty (no occupied voxels) | Summary rebuild pass; consistent with `chunk_flags.is_empty` |
| SUM-3 | All 512 bits are one if and only if every bricklet contains at least one occupied voxel | Summary rebuild pass |
| SUM-4 | `summary_version[slot]` matches `chunk_version[slot]` when the summary is fresh | Summary rebuild postcondition |
| SUM-5 | A slot with `chunk_resident_flags[slot] == 0` has undefined summary content | Pool lifecycle |
| SUM-6 | Total buffer size = `MAX_SLOTS * 16 * 4` bytes | Buffer creation |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| Each u32 word | `0x00000000 .. 0xFFFFFFFF` | Any bit pattern is valid — represents bricklet occupancy |
| `slot_index` | `0 .. MAX_SLOTS-1` | Out-of-range = buffer overrun |
| `bx, by, bz` | `0 .. 7` | Bricklet coordinates within the 8x8x8 grid |
| `bit_index` | `0 .. 511` | `bx * 64 + by * 8 + bz` |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Summary rebuild pass (I-3) | After occupancy upload or edit | Full 16 u32 per slot, computed from `chunk_occupancy_atlas` |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| DDA traversal (Level 0.5) | R-6 | Per-bricklet bit test during ray march to skip empty 8x8x8 regions |
| Segment stream queries | R-6 | Emit empty segments for entire bricklets without descending to voxel level |
| Debug viz | R-9 | Bricklet occupancy heatmap |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Addressing correctness:** For all (bx, by, bz) in [0, 7]^3, verify the bit read from the computed offset matches expected occupancy.
2. **Empty chunk:** All occupancy zero results in all 512 summary bits zero.
3. **Full chunk:** Every bricklet occupied results in all 512 summary bits set.
4. **Single voxel:** One occupied voxel at (x, y, z) sets exactly the bricklet containing that voxel and no others.
5. **Bricklet isolation:** Writing occupancy in bricklet (bx, by, bz) does not affect any other bricklet's summary bit.

### Property tests (Rust, randomized)

6. **Roundtrip:** Generate random occupancy, compute summary, verify each bricklet bit equals OR-reduction of all voxels in that bricklet.
7. **Consistency with chunk_flags:** If all summary bits are zero, `chunk_flags.is_empty` must be set. If any summary bit is one, `chunk_flags.is_empty` must be clear.
8. **Slot isolation:** Computing summary for slot N does not affect slot N+1 or N-1.

### GPU validation (WGSL compute)

9. **Readback test:** Write known occupancy pattern from CPU, dispatch summary rebuild, readback summary, verify against CPU reference.
10. **Traversal test:** Dispatch bricklet DDA on a known occupancy pattern, verify skip/descend decisions match CPU reference.
