# Dirty Chunks

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Control plane — scheduling and coordination metadata, not world state.

> Per-slot dirty bitset tracking which chunks were modified this frame. Drives the propagation and rebuild pipeline.

---

## Identity

- **Buffer name:** `dirty_chunks`
- **WGSL type:** `array<atomic<u32>>` (bitset, 1 bit per slot)
- **GPU usage:** `STORAGE` (read/write by edit kernels, propagation pass, compaction pass)
- **Binding:** `@group(0) @binding(?)` in edit, propagation, and compaction shaders

---

## Layout

A flat bitset. For `MAX_SLOTS` slots, the buffer contains `ceil(MAX_SLOTS / 32)` × `u32` words.

For **1,024 slots**: 32 × `u32` = **128 bytes**.

```
Bit i in word w represents slot (w * 32 + i).

Bit = 1  →  chunk was written this frame
Bit = 0  →  chunk was not written this frame
```

### Setting a dirty bit (WGSL)

```
let word_index = slot >> 5u;
let bit_mask   = 1u << (slot & 31u);
atomicOr(&dirty_chunks[word_index], bit_mask);
```

### Testing a dirty bit (WGSL)

```
let word_index = slot >> 5u;
let bit_mask   = 1u << (slot & 31u);
let is_dirty   = (atomicLoad(&dirty_chunks[word_index]) & bit_mask) != 0u;
```

### Companion structures

The following per-slot control plane buffers provide finer-grained dirty tracking alongside `dirty_chunks`:

| Buffer | Layout | Description |
|---|---|---|
| `dirty_subregions[slot * 16 .. slot * 16 + 15]` | 16 × `u32` per slot (512 bits), atomic | Per-slot subregion grid mapping to 8³ bricklets. Bit j = 1 means bricklet j was modified. |
| `boundary_touch_mask[slot]` | 1 × `u32` per slot, atomic | 6 low bits encode which faces were touched: bit 0 = -X, bit 1 = +X, bit 2 = -Y, bit 3 = +Y, bit 4 = -Z, bit 5 = +Z. |

---

## UNDERSPECIFIED

> **Reset timing.** The narrative says "reset between frames" but does not specify whether the reset happens before edit kernels run or after the propagation pass completes. This must be resolved before implementation. Candidate orderings:
>
> - **Option A:** Reset at frame start, before edit dispatch. Edit kernels and propagation see a clean slate.
> - **Option B:** Reset after propagation completes, before next frame's edits. Propagation reads the full dirty set from the previous frame.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| DRT-1 | `dirty_chunks` is all-zero at frame start (after reset) | Frame reset pass (clearBuffer or zeroing compute) |
| DRT-2 | Bit is set if and only if at least one voxel in that slot was modified since last reset | Edit kernels set bits; no other path sets bits |
| DRT-3 | `boundary_touch_mask[slot]` bits are a subset of the 6 face flags `{0x01, 0x02, 0x04, 0x08, 0x10, 0x20}` | Edit kernels only `atomicOr` valid face bits |
| DRT-4 | `dirty_subregions[slot]` bits correspond to the same 8³ bricklet grid as `occupancy_summary` | Layout definition shared between both buffers |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| `dirty_chunks[w]` | `0x00000000 .. 0xFFFFFFFF` | Any bit pattern valid — each bit is an independent flag |
| `boundary_touch_mask[slot]` | `0x00 .. 0x3F` | Only low 6 bits meaningful; upper 26 bits must be 0 |
| `dirty_subregions[slot * 16 + k]` | `0x00000000 .. 0xFFFFFFFF` | 512 bits per slot total (8³ = 512 bricklets) |
| `slot` | `0 .. MAX_SLOTS-1` | Out-of-range = buffer overrun |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Edit kernels | Runtime voxel edits | `atomicOr` on `dirty_chunks`, `dirty_subregions`, and `boundary_touch_mask` per modified voxel |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Propagation pass | Post-edit | Reads `dirty_chunks` + `boundary_touch_mask` to determine which slots and neighbors need stale flags set |
| Compaction pass | Post-propagation | Reads stale bitsets (written by propagation) to build rebuild work queues |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Bit addressing:** For all slot indices in [0, MAX_SLOTS), verify that setting bit for slot S results in exactly bit `(S & 31)` of word `(S >> 5)` being set.
2. **Reset clears all:** After setting random bits, clear buffer, verify all words are zero.
3. **Boundary mask range:** Verify that edit kernels only produce face bit values within `{0x01, 0x02, 0x04, 0x08, 0x10, 0x20}`.

### Property tests (Rust, randomized)

4. **Set/test roundtrip:** For a random subset of slots, set dirty bits, then test each slot — dirty iff it was in the set.
5. **Slot isolation:** Setting dirty bit for slot S does not affect any other slot's dirty status.
6. **Subregion consistency:** For random voxel edits, verify the bricklet bit in `dirty_subregions` corresponds to the correct 8³ sub-block.

### GPU validation (WGSL compute)

7. **Atomic contention test:** Dispatch many workgroups all setting different slots dirty; readback and verify the union of all expected bits is set with no spurious bits.
8. **Propagation input test:** Set known dirty pattern, dispatch propagation pass, verify stale flags are set for the correct slots and their face-adjacent neighbors.
