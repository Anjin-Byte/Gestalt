# Material-Aware Greedy Merge

**Type:** spec
**Status:** current
**Date:** 2026-03-31
**Depends on:** [chunk-index-buf](data/chunk-index-buf.md), [chunk-palette](data/chunk-palette.md), [material-system](material-system.md), [R-1-mesh-rebuild](stages/R-1-mesh-rebuild.md)

> How the greedy mesher resolves per-voxel material identity and uses it to enforce merge boundaries. Connects the palette protocol to the R-1 inner loop.

---

## The Problem

Binary greedy meshing as implemented in Phase 1 operates on occupancy alone. It merges any adjacent coplanar visible faces into maximal rectangular quads, regardless of material. This produces correct geometry — but wrong shading. A quad spanning two materials gets one material_id in its vertex, which means one of the two materials is silently dropped.

The fix is not complex in concept: stop merging when materials differ. The difficulty is in the execution. The merge loop is the hottest code in the pipeline — 372 GPU threads per chunk, each sweeping a 62x62 grid. Every new memory access in the inner loop multiplies across millions of iterations.

---

## The Constraint: GPU Register Pressure

The current mesh_rebuild.wgsl shader uses two private-memory bitmaps per thread:

```
var<private> processed: array<u32, 121>;  // 484 bytes
var<private> visible:   array<u32, 121>;  // 484 bytes
                                          // Total: 968 bytes
```

GPU register files are typically ~1 KB per thread. Both bitmaps already spill to VRAM scratch memory. Adding a per-cell material array would be catastrophic:

| Approach | Private memory per thread | Feasibility |
|---|---|---|
| Current (no material) | 968 bytes (2 × 121 u32) | Works, spills to scratch |
| Pre-computed material grid (u8) | 968 + 3844 bytes | 4.8 KB — exceeds all practical register budgets |
| Pre-computed material grid (u16) | 968 + 7688 bytes | 8.6 KB — completely unworkable |

Pre-computing a per-cell material identity in private memory is not viable. The merge loop must look up material on-demand from storage buffers.

---

## The Design: On-Demand Material Lookup

Instead of pre-computing material identity for the entire 62x62 slice, the shader reads material identity from the `chunk_index_buf` and `chunk_palette_buf` storage buffers each time a merge candidate is evaluated.

### Data Path

For a voxel at padded coordinates (px, py, pz) in slot S:

```
1. voxel_index = px * 4096 + py * 64 + pz

2. bpe = (palette_meta[S] >> 16) & 0xFF

3. bit_offset  = voxel_index * bpe
   word_index  = bit_offset / 32
   bit_within  = bit_offset % 32
   slot_base   = S * INDEX_BUF_WORDS_PER_SLOT
   mask        = (1 << bpe) - 1
   palette_idx = (index_buf_pool[slot_base + word_index] >> bit_within) & mask

4. pal_word = palette_buf[S * PALETTE_WORDS_PER_SLOT + (palette_idx >> 1)]
   shift    = (palette_idx & 1) * 16
   material_id = (pal_word >> shift) & 0xFFFF
```

Three storage buffer reads total: `palette_meta` (step 2), `index_buf_pool` (step 3), `palette_buf` (step 4).

### Where Lookups Happen in the Merge Loop

```
for each visible cell (primary, secondary):
    seed_mat = read_material_id(slot, px, pz, y)    // 1 lookup: the seed

    // Width extension
    for candidate in primary+1 .. CS:
        if !visible(candidate): break
        cand_mat = read_material_id(slot, ...)       // 1 lookup per width candidate
        if cand_mat != seed_mat: break

    // Height extension
    for row in secondary+1 .. CS:
        for col in primary .. primary+width:
            if !visible(col, row): break to outer
            cand_mat = read_material_id(slot, ...)   // 1 lookup per height candidate
            if cand_mat != seed_mat: break to outer
```

### Cost Model

In the **single-material case** (bpe=1, palette_size ≤ 2), every lookup returns the same material. Merges proceed identically to the current no-material path. The additional cost is the lookup itself: 3 buffer reads per candidate cell.

In the **multi-material case**, merges terminate earlier at material boundaries. This means fewer candidate evaluations per quad but more quads total. The net effect is:
- More quads emitted (smaller rectangles)
- More atomicAdd contention on draw_meta counters
- More vertex/index writes
- Shorter merge extension loops (fewer wasted iterations)

The balance depends on material distribution. For the common case (1–4 materials per chunk, materials in contiguous regions), the additional lookup cost is negligible compared to the occupancy and visibility bitmap reads already happening.

### Why `palette_meta` Can Be Read Once Per Thread

`bits_per_entry` is uniform across all voxels in a slot. The shader can read it once at the top of the entry point and store it in a register:

```wgsl
let bpe = get_bpe(slot);
```

This avoids re-reading `palette_meta` on every lookup. Only `index_buf_pool` and `palette_buf` are read per-voxel.

### Why Not a Material Bitmap

An alternative to on-demand lookup: pre-compute a 62x62 bitmap per material in the slice, then check material membership during merge. This fails for the same register pressure reason — one bitmap per material means N × 121 u32 words of private memory, where N = palette_size. Even for N=2, this doubles the existing bitmap pressure.

A single "material boundary" bitmap (1 where adjacent materials differ) would also work conceptually, but computing it requires the same per-voxel material lookups we're trying to avoid — it just moves the cost from the merge loop to a precompute pass.

### Why Not Shared Memory

Workgroup shared memory is limited to ~16 KB on most GPUs. The current shader uses 256 × MAX_ACTIVE_TRIS = ... actually, mesh_rebuild doesn't use shared memory at all (it uses private bitmaps). Loading material data into shared memory would require:
- 62 × 62 = 3844 bytes per slice (u8 per cell)
- Only one thread per workgroup (each thread handles one slice)
- No sharing benefit — each thread processes a different slice

Shared memory is not useful here because there's no cross-thread data reuse.

---

## Bitpacking Decode: Edge Cases

### Cross-Word Entries

When `voxel_index * bpe` is not aligned to 32-bit boundaries, a single palette_idx may span two u32 words. This happens when:

```
bit_within + bpe > 32
```

For bpe ∈ {1, 2, 4, 8}, this can only occur at bpe=8 when bit_within=24: the 8-bit value starts in the last byte and... actually:
- bpe=1: bit_within is 0–31, entry fits in 1 bit → never spans
- bpe=2: bit_within is 0,2,4,...,30 → max bit_within + bpe = 32 → never spans
- bpe=4: bit_within is 0,4,8,...,28 → max bit_within + bpe = 32 → never spans
- bpe=8: bit_within is 0,8,16,24 → max bit_within + bpe = 32 → never spans

Because bpe is always a power of two that divides 32, entries are always aligned within a single u32 word. **No cross-word handling is needed.** This is a key advantage of restricting bpe to {1, 2, 4, 8}.

### bpe=0

A slot with palette_size=0 is invalid (PAL-3 requires palette_size ≥ 1). A slot with palette_size=1 uses bpe=1 (one bit per voxel, all indices are 0). The shader should never encounter bpe=0, but a defensive check returns palette index 0 if it does.

---

## The Voxel Index Formula

The spec uses `voxel_index = x * 64 * 64 + y * 64 + z` (x-major). This matches the occupancy atlas column layout where column_index = x * CS_P + z, but the index buffer is a flat 3D array, not column-major. The distinction matters:

- **Occupancy atlas:** column-major, u64 per (x, z) column, bit y → fast column scans for traversal
- **Index buffer:** x-major flat array, one entry per voxel → simple linear indexing for material lookup

These are different layouts for different access patterns. The occupancy atlas is optimized for Y-column bitscans. The index buffer is optimized for random-access voxel lookup.

The mesher accesses both: occupancy for visibility (column-based), index_buf for material (voxel-based). They use different addressing and that's correct.

---

## Impact on R-1 Outputs

### Vertex Format

No change to the vertex format. The existing 16-byte vertex already contains material_id in the u32 packed word (bits 31:24 of the normal_material field). Currently hardcoded to 1. After this change, it contains the actual global MaterialId from the palette.

### Quad Count

Multi-material chunks will produce more quads than single-material chunks of the same occupancy. A worst-case alternating checkerboard of two materials produces zero merging — every voxel face is its own 1x1 quad. This is correct behavior: material boundaries are geometric features that must be preserved.

### Pool Limits

The existing fixed allocation (MAX_VERTS_PER_CHUNK = 16384, MAX_INDICES_PER_CHUNK = 24576) may be insufficient for chunks with many material boundaries. The overflow handling (F3 in mesh_rebuild.wgsl) already clamps counts — quads that exceed the limit are silently dropped. This is acceptable for Phase 2. Phase 5 optimization will address this if it becomes a problem with real content.

---

## Companion CPU Implementation

The CPU reference mesher (mesh_cpu.rs) must implement identical logic:

1. Same `read_palette_index` and `read_material_id` functions using the same bitpacking formula
2. Same merge boundary checks at the same points in the merge loop
3. Same material_id assignment on emitted quads

The CPU implementation serves as:
- **Test oracle:** GPU output compared against CPU output for the same input → bit-exact agreement
- **Native testing:** All material merge logic is testable via `cargo test` without a GPU

---

## What This Document Locks In

1. **Material lookup is on-demand, not pre-computed.** Register pressure makes pre-computation infeasible.
2. **Three buffer reads per voxel lookup.** palette_meta (cached in register), index_buf_pool (1 read), palette_buf (1 read).
3. **Merge terminates at material boundaries.** Width and height extension both check material equality.
4. **bpe is always a power of two dividing 32.** No cross-word bitpacking decode needed.
5. **The voxel index formula is x-major (x * 4096 + y * 64 + z).** Different from the column-major occupancy layout.
6. **INDEX_BUF_WORDS_PER_SLOT is sized for worst-case bpe=8.** 65536 u32 words = 256 KB per slot.
7. **The CPU reference implements identical logic** and serves as the correctness oracle.

---

## See Also

- [chunk-index-buf](data/chunk-index-buf.md) — bitpacking layout, invariants IDX-1 through IDX-5
- [chunk-palette](data/chunk-palette.md) — palette layout, invariants PAL-1 through PAL-6
- [material-system](material-system.md) — global material table, palette protocol, MaterialEntry layout
- [R-1-mesh-rebuild](stages/R-1-mesh-rebuild.md) — stage spec (updated with material-aware merge preconditions)
- [gpu-chunk-pool](gpu-chunk-pool.md) — buffer allocation (index_buf_pool + palette_meta added to per-slot layout)
- [chunk-field-registry](chunk-field-registry.md) — materials.index_buf and palette_meta field definitions
