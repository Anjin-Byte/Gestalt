# Chunk Index Buffer

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Authoritative data — never derived, written by producers only.

> Per-voxel palette indices bitpacked at variable bit width. Maps every voxel in a chunk to its material via the chunk palette.

---

## Identity

- **Buffer name:** `chunk_index_buf`
- **WGSL type:** `array<u32>` (per slot, bitpacked per-voxel palette indices)
- **GPU usage:** `STORAGE | COPY_DST`
- **Binding:** `@group(0) @binding(TBD)` in material lookup and traversal shaders

### Companion: Palette Metadata

- **Buffer name:** `palette_meta`
- **WGSL type:** `array<u32>` (one u32 per slot)
- **GPU usage:** `STORAGE | COPY_DST`
- **Layout:** Indexed by slot. Each u32 encodes:

| Bits | Field | Type | Description |
|---|---|---|---|
| 0--15 | `palette_size` | u16 | Number of palette entries (1--256) |
| 16--23 | `bits_per_entry` | u8 | Bit width per voxel index (1, 2, 4, or 8) |
| 24--31 | reserved | u8 | Must be 0 (IDX-5) |

---

## Layout

One slot stores 64x64x64 = 262,144 voxel indices, each `bits_per_entry` (bpe) bits wide, packed contiguously into u32 words.

```
For voxel at (x, y, z) within the chunk:
  voxel_index = x * 64 * 64 + y * 64 + z   // 0 .. 262143
  word        = voxel_index * bpe / 32       // which u32 word
  bit_off     = (voxel_index * bpe) % 32     // bit offset within word
  mask        = (1u << bpe) - 1
  palette_idx = (chunk_index_buf[slot_offset + word] >> bit_off) & mask
```

Where `slot_offset` is the base u32 offset for this slot's index region.

### Buffer size per slot

| bpe | u32 words per slot | Bytes per slot |
|---|---|---|
| 1 | 8,192 | 32,768 |
| 2 | 16,384 | 65,536 |
| 4 | 32,768 | 131,072 |
| 8 | 65,536 | 262,144 |

Formula: `ceil(262144 * bpe / 32)` u32 words per slot.

### Why variable bit width?

Most chunks use only a handful of materials. A chunk with 2 materials needs only 1 bit per voxel (32 KB) instead of 8 bits (256 KB). This 8x reduction in the common case is significant when thousands of chunks are resident.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| IDX-1 | `bits_per_entry` in {1, 2, 4, 8} | Voxelizer postcondition; palette resize validation |
| IDX-2 | Every voxel's `palette_idx` < `palette_size` | Voxelizer output validation; edit kernel bounds check |
| IDX-3 | If a voxel is unoccupied (`opaque_mask` bit == 0), its `palette_idx` is undefined | Consumers must check occupancy before reading index |
| IDX-4 | Buffer size exactly matches `ceil(262144 * bpe / 32)` u32 words per slot | Buffer creation; palette resize reallocation |
| IDX-5 | `palette_meta` bits 24--31 are reserved and must be 0 | Write mask on all palette_meta producers |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| `palette_idx` (per voxel) | `0 .. palette_size-1` | Only meaningful when occupancy bit is set |
| `palette_size` | `1 .. 256` | Stored in `palette_meta[slot]` bits 0--15 |
| `bits_per_entry` | `1, 2, 4, 8` | Stored in `palette_meta[slot]` bits 16--23 |
| `voxel_index` | `0 .. 262143` | `x * 4096 + y * 64 + z` where x, y, z in [0, 63] |
| Each u32 word | `0x00000000 .. 0xFFFFFFFF` | Any bit pattern valid — undefined bits for unoccupied voxels |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Voxelizer (I-1) | Mesh load / procedural gen | Full 64x64x64 index buffer and palette_meta for new chunks |
| Edit kernels | Runtime voxel material changes | Individual voxel indices within the bitpacked buffer |
| Palette resize operation | When edit introduces a new material that exceeds current bpe capacity | Re-encodes entire slot at new bpe; updates palette_meta |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Fragment material lookup | R-5 | Reads palette_idx for the hit voxel, then indexes into chunk_palette_buf |
| Traversal hit material lookup | R-6 | During DDA ray march, resolves hit voxel to palette_idx for material-dependent logic |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Addressing correctness:** For all (x, y, z) in [0, 63]^3 and each bpe in {1, 2, 4, 8}, verify the palette_idx read from the computed offset matches the value written.
2. **Bit isolation:** Writing palette_idx at voxel (x, y, z) does not modify any other voxel's index.
3. **Boundary bpe values:** At palette_size transitions (2->3, 4->5, 16->17), verify bpe scales correctly.
4. **palette_meta encoding:** Write and read back palette_size and bits_per_entry — both match.
5. **Reserved bits:** Verify palette_meta bits 24--31 are always 0 after any write.

### Property tests (Rust, randomized)

6. **Roundtrip:** Generate random palette indices (within palette_size) for all 262,144 voxels, pack at each bpe, unpack — every index matches.
7. **Resize correctness:** Re-encode a slot from bpe=2 to bpe=4, verify all voxel indices are preserved.
8. **Slot isolation:** Writing to slot N does not affect slot N+1 or N-1.

### GPU validation (WGSL compute)

9. **Readback test:** Write known index pattern from CPU, dispatch compute shader that reads every voxel index, verify via readback buffer.
10. **Cross-reference test:** For each occupied voxel, verify palette_idx < palette_size and that the corresponding palette entry is a valid MaterialId.
