# Stale Flags

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Control Plane — staleness tracking for derived products.

> Bitsets tracking which derived products are stale for each chunk slot. Written by the propagation pass (or mesh rebuild pass for meshlets); cleared by the corresponding rebuild pass after committing new artifacts.

---

## Identity

- **Buffer names:** `stale_mesh`, `stale_summary`, `stale_lighting`, `stale_meshlet`
- **WGSL type:** `array<u32>` (one bit per slot, same layout as `dirty_chunks`)
- **GPU usage:** `STORAGE`
- **Binding:** control-plane group, read by compaction pass, written by propagation pass (or mesh rebuild pass for `stale_meshlet`)

---

## Layout

Each bitset occupies `ceil(MAX_SLOTS / 32)` u32 words. One bit per chunk slot.

```
For slot index S:
  word_index  = S >> 5          // S / 32
  bit_within  = S & 31          // S % 32

  is_stale = (stale_X[word_index] >> bit_within) & 1
```

Total size per bitset: `ceil(MAX_SLOTS / 32) * 4` bytes.

### Four Bitsets

| Bitset | Written by | Cleared by | Meaning |
|---|---|---|---|
| `stale_mesh` | Propagation pass | Mesh rebuild pass (R-1) after committing vertex/index data | Chunk mesh is out of date with respect to occupancy |
| `stale_summary` | Propagation pass | Summary rebuild pass (I-3) after committing flags, summary, AABB | Occupancy summaries are out of date |
| `stale_lighting` | Propagation pass | Lighting update pass after invalidating GI/probe caches | GI/lighting caches are out of date |
| `stale_meshlet` | Mesh rebuild pass (R-1) | Meshlet rebuild pass after committing meshlet descriptors | Meshlet cluster data is out of date with respect to the mesh |

`stale_meshlet` is special: it is not written by the propagation pass. It is written by the mesh rebuild pass after committing a new mesh, because meshlet staleness is a consequence of mesh staleness, not a direct consequence of voxel edits.

### Propagation Pass Logic

The propagation pass runs after edit kernels, before any rebuild work:

```
Input:  dirty_chunks, boundary_touch_mask
Output: stale_mesh, stale_summary, stale_lighting

For each dirty chunk slot S:
  1. Set stale_mesh[S]
  2. Set stale_summary[S]
  3. Set stale_lighting[S] (within configurable radius)
  4. If boundary_touch_mask[S] has any face bits set:
     - For each touched face, look up adjacent slot A
     - Set stale_mesh[A]
     - Set dirty bit for A in dirty_chunks
```

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| STL-1 | A bit in `stale_mesh` is 1 only if `chunk_occupancy_atlas` (or a neighbor's boundary) was written since the last mesh rebuild for that slot | Propagation pass sets; mesh rebuild clears |
| STL-2 | A bit in `stale_summary` is 1 only if `chunk_occupancy_atlas` was written since the last summary rebuild for that slot | Propagation pass sets; summary rebuild clears |
| STL-3 | A bit in `stale_lighting` is 1 only if occupancy changed within a configurable radius since the last lighting update for that slot | Propagation pass sets; lighting pass clears |
| STL-4 | A bit in `stale_meshlet` is 1 only if the mesh rebuild pass committed a new mesh for that slot since the last meshlet rebuild | Mesh rebuild pass sets; meshlet rebuild clears |
| STL-5 | `stale_meshlet` is never written by the propagation pass | Propagation pass excludes `stale_meshlet` from its output set |
| STL-6 | A rebuild pass must clear the stale bit only after successfully committing the new artifact and passing the version check | Rebuild pass postcondition |
| STL-7 | If `chunk_resident_flags[slot] == 0`, stale bits for that slot are undefined | Pool lifecycle |
| STL-8 | Total bitset size = `ceil(MAX_SLOTS / 32) * 4` bytes per bitset | Buffer creation |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| Each u32 word | `0x00000000 .. 0xFFFFFFFF` | Any bit pattern is valid — represents staleness |
| `slot_index` | `0 .. MAX_SLOTS-1` | Out-of-range = buffer overrun |
| `word_index` | `0 .. ceil(MAX_SLOTS/32)-1` | Derived from slot_index |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Propagation pass | After edit kernels, before rebuild work | `stale_mesh`, `stale_summary`, `stale_lighting` bits for dirty chunks and their neighbors |
| Mesh rebuild pass (R-1) | After committing a new mesh | `stale_meshlet` bit for the rebuilt slot |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Compaction pass | After propagation | Scans all four bitsets, appends slot indices to corresponding rebuild queues |
| CPU (Stage 2 only) | Queue count readback | Indirectly — stale bits drive queue counts read by CPU for budget decisions |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Addressing correctness:** For all slot indices in [0, MAX_SLOTS), verify the bit read from the computed word/bit offset matches the bit written.
2. **Bit isolation:** Setting stale for slot S does not modify any other slot's bit.
3. **Propagation coverage:** After a dirty chunk with boundary_touch_mask +X, verify `stale_mesh` is set for both the dirty slot and the +X neighbor slot.
4. **Meshlet independence:** Verify that the propagation pass does not set `stale_meshlet` — only `stale_mesh`, `stale_summary`, `stale_lighting`.
5. **Clear semantics:** After a rebuild pass clears a stale bit, verify the bit reads as 0.

### Property tests (Rust, randomized)

6. **Roundtrip:** Generate random dirty_chunks bitset, run propagation, verify stale_mesh is a superset of dirty_chunks.
7. **Neighbor expansion:** For random boundary_touch_mask patterns, verify the correct neighbor slots are marked stale.
8. **No spurious clears:** A rebuild pass that fails the version check does not clear the stale bit.

### GPU validation (WGSL compute)

9. **Readback test:** Write known dirty pattern from CPU, dispatch propagation, readback stale bitsets, verify against CPU reference.
10. **Compaction test:** Write known stale pattern, dispatch compaction, verify rebuild queue contents match the set bits.
