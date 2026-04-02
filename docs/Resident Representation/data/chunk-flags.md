# Chunk Flags

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Derived — rebuilt by I-3 summary rebuild from `chunk_occupancy_atlas` and `chunk_palette`.

> Packed per-slot bitfield summarizing chunk properties. Enables fast skip decisions without reading full occupancy.

---

## Identity

- **Buffer name:** `chunk_flags`
- **WGSL type:** `array<u32>`
- **GPU usage:** `STORAGE | COPY_DST`
- **Binding:** Read by R-1, R-2, R-4, R-6; written by I-3

---

## Layout

One `u32` per slot. Bit assignments:

```
Bit 0:  is_empty           — no occupied voxels in the chunk
Bit 1:  is_fully_opaque    — all 64³ voxels occupied
Bit 2:  has_emissive        — at least one voxel has an emissive material
Bit 3:  is_resident         — slot is currently allocated and valid
Bit 4:  stale_mesh          — occupancy changed since last mesh rebuild
Bit 5:  stale_summary       — occupancy changed since last summary rebuild
Bit 6:  stale_lighting      — occupancy or material changed since last cascade build
Bit 7:  has_transparency    — at least one voxel has opacity < 1.0 (future)
Bits 8-31: reserved (must be 0)
```

Total: **4 bytes per slot**. For 1024 slots: **4 KB**.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| FLG-1 | `is_empty == (popcount(all occupancy words for this slot) == 0)` | I-3 postcondition |
| FLG-2 | `is_fully_opaque == (all occupancy words == 0xFFFFFFFF for inner region)` | I-3 postcondition |
| FLG-3 | `has_emissive == (any palette entry maps to a material with emissive > 0 in material_table)` | I-3 postcondition |
| FLG-4 | `is_resident == 1` if and only if `chunk_resident_flags[slot] == 1` | Pool manager + I-3 |
| FLG-5 | `stale_mesh == 1` implies `chunk_version[slot] > version at last mesh rebuild` | Edit protocol |
| FLG-6 | Reserved bits 8-31 are always 0 | I-3 clears them; edit protocol preserves them |
| FLG-7 | If `is_empty == 1`, then `stale_mesh` is irrelevant (empty chunks have no mesh) | R-1 precondition check |

---

## Derived From

| Source | Fields derived |
|---|---|
| `chunk_occupancy_atlas[slot]` | `is_empty`, `is_fully_opaque` |
| `chunk_palette[slot]` + `material_table` | `has_emissive`, `has_transparency` |
| `chunk_resident_flags[slot]` | `is_resident` |
| Edit protocol dirty bits | `stale_mesh`, `stale_summary`, `stale_lighting` |

---

## Consumers

| Consumer | Stage | Which bits | Purpose |
|---|---|---|---|
| Greedy mesher | R-1 | `stale_mesh`, `is_empty` | Skip clean/empty chunks |
| Depth prepass | R-2 | `is_empty`, `is_resident` | Skip empty/unloaded chunks |
| Occlusion cull | R-4 | `is_empty`, `is_resident` | Skip from cull input |
| DDA traversal | R-6 | `is_empty`, `is_resident` | Skip empty chunks in ray march |
| Cascade build | R-6 | `has_emissive` | Identify light sources |
| Debug viz | R-9 | All bits | Chunk state coloring |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Empty detection:** Generate an all-zero occupancy atlas for a slot. Run flag derivation. Verify `is_empty == 1`, `is_fully_opaque == 0`.
2. **Full detection:** Generate an all-ones occupancy atlas. Verify `is_fully_opaque == 1`, `is_empty == 0`.
3. **Emissive detection:** Set one palette entry to an emissive material. Verify `has_emissive == 1`. Remove it. Verify `has_emissive == 0`.
4. **Stale propagation:** Set a voxel in a clean chunk. Verify `stale_mesh == 1`, `stale_summary == 1`.
5. **Reserved bits:** After every flag computation, verify bits 8-31 are 0.

### Property tests (Rust, randomized)

6. **is_empty consistency:** For 1000 random occupancy patterns, verify `is_empty` matches `popcount == 0`.
7. **is_fully_opaque consistency:** Verify `is_fully_opaque` matches all inner words being `0xFFFFFFFF`.
8. **Stale lifecycle:** Edit → verify stale set → rebuild → verify stale cleared → no edit → verify still clean.

### Cross-structure tests

9. **FLG-1 vs occupancy:** For every slot, `is_empty` must agree with a CPU scan of `chunk_occupancy_atlas[slot]`.
10. **FLG-4 vs resident_flags:** For every slot, `is_resident` bit must match `chunk_resident_flags[slot]`.
