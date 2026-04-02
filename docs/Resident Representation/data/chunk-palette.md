# Chunk Palette

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Authoritative data — never derived, written by producers only.

> Per-chunk ordered list of unique MaterialIds. The palette maps compact per-voxel indices back to actual material identifiers.

---

## Identity

- **Buffer name:** `chunk_palette_buf`
- **WGSL type:** `array<u32>` (per slot, u16 MaterialIds packed 2 per u32)
- **GPU usage:** `STORAGE | COPY_DST`
- **Binding:** `@group(0) @binding(TBD)` in material lookup and emissive scan shaders

---

## Layout

Each palette slot stores 1--256 unique MaterialIds as u16 values, packed two per u32 word.

```
For palette entry i within a slot:
  word_index  = i >> 1                       // 2 entries per u32
  shift       = (i & 1) * 16                 // low or high half
  palette_entry[i] = (chunk_palette_buf[slot_offset + word_index] >> shift) & 0xFFFF
```

Where `slot_offset` is the base u32 offset for this slot's palette region.

### Palette sizing

| Palette size | Bits per entry (in index buf) | Max u32 words |
|---|---|---|
| 1--2 entries | 1 bit | 1 |
| 3--4 entries | 2 bits | 2 |
| 5--16 entries | 4 bits | 8 |
| 17--256 entries | 8 bits | 128 |

Bit width auto-scales based on the number of unique materials present in the chunk. The bit width determines how per-voxel indices are packed in `chunk_index_buf`.

### Why packed u16?

MaterialIds are 16-bit values. Packing two per u32 halves the storage footprint and keeps palette reads to at most 128 u32 words for the maximum 256-entry case. The low-half-first ordering matches little-endian convention and avoids cross-word entries.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| PAL-1 | Every palette entry is a valid MaterialId in [0, MAX_MATERIALS-1] | Voxelizer output validation |
| PAL-2 | Palette contains no duplicate MaterialIds | Voxelizer dedup pass; edit kernel merge |
| PAL-3 | `palette_size` in [1, 256] | Voxelizer postcondition; edit kernel bounds check |
| PAL-4 | `palette[0]` is always the dominant material (highest voxel count) | Voxelizer sort; edit kernel re-sort on dominance change |
| PAL-5 | Palette entry 0 (`MATERIAL_EMPTY`) is never stored in the palette — empty means no voxel, not a material | Voxelizer filter; edit kernel precondition |
| PAL-6 | A slot with `chunk_resident_flags[slot] == 0` has undefined palette content | Pool lifecycle |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| Each u16 MaterialId | `0x0001 .. MAX_MATERIALS-1` | 0x0000 (`MATERIAL_EMPTY`) is excluded by PAL-5 |
| `palette_size` | `1 .. 256` | Stored in `palette_meta[slot]` bits 0--15 |
| `slot_offset` | Varies | **UNDERSPECIFIED:** GPU slot_offset calculation for variable-length palettes across slots is not yet defined |

---

## Underspecified

- **GPU slot_offset calculation:** Because palettes are variable-length (1--256 entries), the per-slot offset into `chunk_palette_buf` is not a simple multiply. Options under consideration: (a) fixed max-size allocation (128 u32 per slot, wasteful), (b) prefix-sum offset table, (c) indirect buffer. This must be resolved before the buffer can be bound in shaders.

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Voxelizer (I-1) | Mesh load / procedural gen | Full palette for new chunks — deduplicated, sorted by dominance |
| Edit kernels | Runtime voxel material changes | Append new MaterialId or re-sort on dominance shift |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Emissive scan | I-3 | Reads palette to identify emissive MaterialIds for light source extraction |
| Material-aware merge | R-1 | Reads palette during greedy meshing to determine face materials |
| Fragment lookup | R-5 | Maps per-voxel palette index → MaterialId for shading |
| Emissive hit test | R-6 | During DDA traversal, resolves hit voxel's MaterialId to check emissive flag |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Pack/unpack roundtrip:** For palette sizes 1--256, write MaterialIds via the packing formula, read back every entry — all match.
2. **No duplicates:** After voxelizer output, verify palette contains no repeated MaterialIds.
3. **Dominance ordering:** Verify palette[0] has the highest voxel count in the chunk.
4. **Empty exclusion:** Verify MATERIAL_EMPTY (0x0000) never appears in any palette entry.
5. **Boundary palette sizes:** Test edge cases at 1, 2, 3, 4, 5, 16, 17, and 256 entries.

### Property tests (Rust, randomized)

6. **Roundtrip with random materials:** Generate random sets of 1--256 unique MaterialIds, pack into buffer, unpack — all match.
7. **Slot isolation:** Writing palette to slot N does not corrupt slot N+1 or N-1.
8. **Bit-width consistency:** Verify that palette_size correctly implies the bits_per_entry stored in `palette_meta`.

### GPU validation (WGSL compute)

9. **Readback test:** Write known palette from CPU, dispatch compute shader that unpacks every entry, verify via readback buffer.
10. **Cross-reference test:** For each occupied voxel, verify its palette_idx (from `chunk_index_buf`) maps to a valid palette entry.
