# Palette-Based Material Storage Migration Plan

## Why Change the Approach
The current `BinaryChunk` stores a full `64^3` `u16` material array, which is fast and simple but heavy on memory. This dominates per-chunk memory and caps the number of resident chunks. A palette + bitpacked index buffer makes storage proportional to the number of unique materials per chunk, which is typically small.

### Memory footprint (per chunk)
- Current:
  - `opaque_mask`: 32 KiB
  - `materials`: 512 KiB
  - Total: 544 KiB
  - Reference: `crates/greedy_mesher/src/core.rs`
- Palette + bitpacked indices:
  - `opaque_mask`: 32 KiB (unchanged)
  - indices: `64^3 * b / 8` bytes, where `b = ceil(log2(n))`
  - palette: `n * 2` bytes (`u16` each)

### Example reductions (materials only, not counting mask)
- `n=4` -> `b=2` -> 64 KiB (~88% reduction)
- `n=16` -> `b=4` -> 128 KiB (~75% reduction)
- `n=256` -> `b=8` -> 256 KiB (~50% reduction)
- `n=4096` -> `b=12` -> 384 KiB (~25% reduction)
- `n=65536` -> `b=16` -> 512 KiB (0% reduction)

## Current System Overview

### Data types
- `MaterialId = u16`, `MATERIAL_EMPTY = 0`
  - `crates/greedy_mesher/src/core.rs`
- `BinaryChunk`:
  - `opaque_mask: [u64; 64*64]` (1 bit per voxel)
  - `materials: [u16; 64^3]`
  - `crates/greedy_mesher/src/core.rs`

### Supporting functions
- `BinaryChunk::set/clear/get_material/is_solid`
  - O(1) index math + array access
  - `crates/greedy_mesher/src/core.rs`
- Greedy merge looks up materials only for visible faces:
  - `crates/greedy_mesher/src/merge/x_faces.rs`
  - `crates/greedy_mesher/src/merge/y_faces.rs`
  - `crates/greedy_mesher/src/merge/z_faces.rs`
- `Chunk` wraps `BinaryChunk` and exposes `get_voxel/set_voxel`
  - `crates/greedy_mesher/src/chunk/chunk.rs`
- `ChunkManager` handles world->voxel mapping and dirty tracking
  - `crates/greedy_mesher/src/chunk/manager.rs`

### Time/space complexity (current)
- Material lookup: O(1)
- Space: O(64^3) fixed per chunk

## Proposed Palette + Bitpacked Materials

### Data structure requirements
- Keep `opaque_mask` unchanged for fast solid checks.
- Replace `materials: [u16; 64^3]` with:
  - `palette: Vec<MaterialId>` (unique materials per chunk)
  - `indices: Vec<u64>` or `Vec<u32>` (bitpacked indices)
  - `bits_per_voxel: u8`

### Lookup and set paths
- `get_material(x,y,z)`:
  1) compute voxel linear index
  2) bit-unpack `bits_per_voxel` from `indices`
  3) return `palette[idx]`
- `set_material(x,y,z,mat)`:
  1) find existing palette index (or insert)
  2) if palette size exceeds `2^bits_per_voxel`, repack entire chunk
  3) bit-pack index into `indices`

### Extra logic introduced
- Palette management (add/search/remove)
- Bitpacking math
- Repack on palette growth (O(n) over chunk voxels)

## Complexity Comparison

### Current
- Get/Set: O(1), very low constant
- Space: fixed 512 KiB for materials

### Palette + bitpacked
- Get: O(1). Must be treated as performance-critical; keep the fast path branchless and cache-friendly.
- Set: O(1) normally; O(n) on palette growth/repack
- Space: O(64^3 * log2(n)) + O(n)

## Implementation Plan (Phased)

### Phase 1: Data type and API changes
- Introduce new struct:
  - `PaletteMaterials { palette, indices, bits_per_voxel }`
- Add `BinaryChunk::get_material` / `set` backed by palette logic
- Keep the same external API so merge code stays unchanged

### Phase 2: Repack logic
- Implement `ensure_capacity(new_palette_len)`:
  - If new `bits_per_voxel` required, allocate new indices buffer
  - Iterate over all voxels to repack indices
- Add fast path if palette index already exists

### Phase 2.1: Efficient repack implementation
- Specialized repack for each `(old_bits, new_bits)` pair (1..16). This keeps inner loops branchless.
- Use `match (old_bits, new_bits)` once per repack and dispatch to a specialized function.
- Prefer repacking over packed words (`u64`) instead of per-voxel structs to reduce overhead.
- Batch palette growth: when inserting many new materials, defer repack and do it once.
- Keep `bits_per_voxel` in local variables and avoid bounds checks inside the hot loop.
- Use `unsafe` with a single upfront bounds check per buffer. If the per-case memory footprint is proven correct, the inner loop can forgo bounds checks safely.

### Phase 2.2: Macro-generated repack specialization
To keep the hot loop branchless, generate per-case repack functions at compile time and dispatch once by `(old_bits, new_bits)`.

Example (shape of generated functions):
```rust
#[inline]
fn repack_3_5(src: &[u64], dst: &mut [u64]) {
    let total = 64usize * 64 * 64;
    let mut i = 0usize;
    while i < total {
        let idx = get_idx::<3>(src, i);
        set_idx::<5>(dst, i, idx);
        i += 1;
    }
}
```

Dispatch once per repack:
```rust
fn repack_dispatch(old_bits: u8, new_bits: u8, src: &[u64], dst: &mut [u64]) {
    match (old_bits, new_bits) {
        (3, 5) => repack_3_5(src, dst),
        // ... all ordered pairs, old != new
        _ => unreachable!("invalid repack"),
    }
}
```

### Phase 3: Serialization and interop
- Update any WASM bindings that expose chunk memory
- Ensure `ChunkManager::populate_dense` remains efficient (batch insert, single repack at end)

## Invariants of the New Structure
1) Palette index validity: all packed indices are < `palette.len()`
2) Empty voxel consistency: palette index 0 equals `MATERIAL_EMPTY`
3) Opaque mask consistency: mask bit is 1 iff material != `MATERIAL_EMPTY`
4) Bit width correctness: `bits_per_voxel = ceil(log2(palette.len()))`, minimum 1
5) Repack preservation: repacking must preserve all voxel materials exactly

## Testing Strategy (Rigorous)

### Unit tests
- `get/set` roundtrip for random voxels
- palette growth boundary (1->2->4->8 materials)
- repack preserves values across full chunk
- `opaque_mask` consistency after batch edits
- empty voxel handling (material 0 -> mask cleared)

### Property tests
- Random voxel edits + full chunk scan:
  - materials returned must match expected model
  - after repack, all materials identical
- Regression tests for greedy merge:
  - compare quads before/after palette change on identical input

### Performance tests
- Measure per-voxel lookup cost in greedy merge loops
- Measure repack time for worst-case palette growth
