# GPU Greedy Mesher Code Review

**Type:** reference
**Date:** 2026-03-24
**File under review:** `crates/wasm_renderer/src/shaders/mesh_rebuild.wgsl`
**CPU reference:** `crates/wasm_renderer/src/mesh_cpu.rs`
**Legacy reference:** `crates/greedy_mesher/src/cull.rs`, `expand.rs`, `merge/*.rs`

> Independent review of the R-1 GPU greedy mesh rebuild compute shader. Each finding is documented with evidence, impact assessment, and recommended fix.

---

## Finding 1: Stale Index Pattern Comment

**Severity:** Low
**Type:** Documentation / maintenance hazard

### Description

Line 426 reads:
```wgsl
// Write 6 indices: [0,1,2, 2,1,3] pattern
```

The actual code (lines 429-434) writes:
```wgsl
index_pool[ib]      = vbase;
index_pool[ib + 1u] = vbase + 1u;
index_pool[ib + 2u] = vbase + 2u;
index_pool[ib + 3u] = vbase;       // ← 0, not 2
index_pool[ib + 4u] = vbase + 2u;  // ← 2, not 1
index_pool[ib + 5u] = vbase + 3u;
```

This is `[0,1,2, 0,2,3]` — the correct pattern (fixed earlier in the session when `[0,1,2, 2,1,3]` caused half the faces to have inverted winding). The comment was not updated with the fix.

### Impact

A future developer reading the comment would believe the pattern is `[2,1,3]` for the second triangle. If they "fix" the code to match the comment, they reintroduce the winding bug — half the faces become invisible due to backface culling.

### Recommendation

Update the comment to match the code:
```wgsl
// Write 6 indices: [0,1,2, 0,2,3] pattern (CCW winding, both triangles face outward)
```

### Evidence

The CPU reference (`mesh_cpu.rs` line 413) uses the same `[0,1,2, 0,2,3]` pattern. The test `index_pattern_correct` (mesh_cpu.rs) validates this. The winding was verified via cross-product for all 6 face directions — see Finding 4 for the full proof.

---

## Finding 2: Redundant Face Cull Recomputation

**Severity:** Medium
**Type:** Performance

### Description

The WGSL shader recomputes `cull_column()` from scratch for every cell during the greedy merge. For a single 62×62 slice, the inner loop (lines 210-435) calls `cull_column` at:

1. The initial visibility check (line 242)
2. Every step of width extension (line 278)
3. Every cell of every step of height extension (line 321)

Each `cull_column` call performs:
- 2 global buffer reads (`read_col` for the column)
- 2 more global buffer reads (`get_neighbor` for the neighbor column)
- Bitwise operations (`and2`, `not2`, `shr1` or `shl1`, `to_usable`)

### Quantitative Impact

For a fully occupied 62×62 slice:
- Initial scan: 62 × 62 = 3,844 calls
- Width extension: average ~31 additional checks per row × 62 rows ≈ 1,922 calls
- Height extension: average ~31 additional checks per column × 62 columns × width ≈ variable but significant

**Worst case:** ~10,000+ `cull_column` calls per slice per thread. With 62 threads × 6 face directions = 372 threads, this is millions of redundant buffer reads per chunk.

### How the Legacy Mesher Handles This

The legacy `cull.rs` pre-computes ALL face masks in a single O(n) pass over columns before merging begins:

```rust
// cull.rs — one pass, all faces computed together
for x in 1..(CS_P - 1) {
    for z in 1..(CS_P - 1) {
        let column = chunk.opaque_mask[column_idx];
        if column == 0 { continue; }
        // Compute all 6 face masks at once
        masks.set(FACE_POS_Y, x, z, (pos_y >> 1) & usable_mask);
        masks.set(FACE_NEG_Y, x, z, (neg_y >> 1) & usable_mask);
        // ... etc
    }
}
```

The merge step then reads from `masks` (a flat array) — no buffer reads, no recomputation.

### Why This Matters on GPU

Each `read_col` call is a global memory load. GPU global memory has high latency (~400 cycles). While the GPU can hide latency via thread interleaving, 372 threads all reading the same buffer region creates cache pressure. Pre-computing face masks into `var<private>` memory would convert global reads into register/local reads.

### Recommended Fix

Pre-compute face masks for all 62 columns in the current slice at the start of each thread, before the merge loop:

```wgsl
// Pre-compute face masks for this slice (62 vec2u values)
var<private> face_masks: array<vec2u, 62>;

// At thread start, before merge loop:
for (var i = 0u; i < CS; i++) {
    // Map i to the column coordinates for this slice + face direction
    var col_px: u32; var col_pz: u32;
    // ... (depends on face direction)
    let col = read_col(slot_offset, col_px, col_pz);
    let nbr = get_neighbor(slot_offset, col_px, col_pz, face);
    face_masks[i] = cull_column(col, nbr, face);
}
```

Then replace all `cull_column` calls in the merge loop with `face_masks[index]` lookups.

**Trade-off:** This adds 62 × `sizeof(vec2u)` = 496 bytes to private memory per thread (on top of the 484-byte processed bitmap). Total: ~980 bytes per thread. This is more private memory pressure but eliminates all redundant global reads.

### Alternative Fix

Use shared memory (`var<workgroup>`) for the face masks since all 62 threads in a workgroup process the same slice set for the same face direction. Thread T could pre-compute face masks for columns it owns, store them in shared memory, then barrier and read any column's mask from shared memory.

---

## Finding 3: Overflow Guard Leaks Counter Space

**Severity:** Medium
**Type:** Correctness

### Description

Lines 342-348:
```wgsl
let vert_claim = atomicAdd(&draw_meta[meta_base + 1u], 4u);
let idx_claim = atomicAdd(&draw_meta[meta_base + 3u], 6u);

// Overflow guard
if vert_claim + 4u > MAX_VERTS_PER_CHUNK || idx_claim + 6u > MAX_INDICES_PER_CHUNK {
    continue;
}
```

`atomicAdd` returns the **previous** value and **always increments** the counter. When the guard triggers, the quad is discarded, but the counters remain inflated. The final counter values in `draw_meta` include the overflowed claims.

### Failure Scenario

1. Thread A claims vertices 16380-16383 (valid, vert_claim=16380)
2. Thread B claims vertices 16384-16387 (overflow, vert_claim=16384 > MAX=16384, guard triggers)
3. Thread B's quad is discarded. No vertices written at 16384-16387.
4. `draw_meta.vertex_count` is now 16388 (16384 + 4 from thread B's atomicAdd).
5. `build_indirect` reads `vertex_count=16388` and generates a draw call for 16388 vertices.
6. The render pass reads vertices 16384-16387, which contain whatever was in the buffer (likely zeros from a previous clear, or stale data from a prior frame).

### Impact

With the current test scene (1104 quads = 4416 vertices, well under 16384), this cannot trigger. It becomes a real problem when:
- MAX_SLOTS chunks are resident and each has complex geometry
- A chunk near the vertex limit gets rebuilt
- Stale geometry from overflowed counters appears as visual artifacts

### Recommended Fix — Option A: Clamp in `build_indirect`

The simplest fix — `build_indirect.wgsl` clamps the count before writing:

```wgsl
let index_count = min(draw_meta[meta_base + 3u], MAX_INDICES_PER_CHUNK);
```

This ensures the draw call never reads past the valid region, even if the counter overflowed. The overflowed quads are still lost (not drawn), but no garbage geometry appears.

### Recommended Fix — Option B: Check before claiming

Replace `atomicAdd` with a load-check-CAS loop:

```wgsl
var vert_claim: u32;
loop {
    vert_claim = atomicLoad(&draw_meta[meta_base + 1u]);
    if vert_claim + 4u > MAX_VERTS_PER_CHUNK {
        // Overflow — skip without incrementing
        break;
    }
    let result = atomicCompareExchangeWeak(&draw_meta[meta_base + 1u], vert_claim, vert_claim + 4u);
    if result.exchanged {
        break;
    }
    // CAS failed — another thread incremented. Retry.
}
```

This is more complex but keeps the counter accurate. Only recommended if counter accuracy matters for diagnostics.

### Recommended Fix — Option C (Pragmatic): Both

Apply Option A (cheap, defensive) AND leave the current atomicAdd code. The clamp in `build_indirect` prevents visual artifacts. The counter in draw_meta is "approximately correct" which is fine for Phase 1.

---

## Finding 4: Vertex Winding Order — Verified Correct

**Severity:** None (verified)
**Type:** Mathematical verification

### Method

For each face direction, extract the 4 corner positions from the WGSL shader, compute the cross product of edges (v0→v1) × (v0→v2) for the first triangle `[0,1,2]` and (v0→v2) × (v0→v3) for the second triangle `[0,2,3]`, and verify both normals point in the expected direction.

### +Y Face (case 0, line 357-367)

```
v0 = (bx,     by, bz)
v1 = (bx,     by, bz+h)
v2 = (bx+w,   by, bz+h)
v3 = (bx+w,   by, bz)

Triangle [0,1,2]: e01=(0,0,h), e02=(w,0,h)
  cross = (0·h - h·0, h·w - 0·h, 0·0 - 0·w) = (0, wh, 0) → +Y ✓

Triangle [0,2,3]: e02=(w,0,h), e03=(w,0,0)
  cross = (0·0 - h·0, h·w - w·0, w·0 - 0·w) = (0, wh, 0) → +Y ✓
```

### -Y Face (case 1, line 369-378)

```
v0 = (bx,     by, bz)
v1 = (bx+w,   by, bz)
v2 = (bx+w,   by, bz+h)
v3 = (bx,     by, bz+h)

Triangle [0,1,2]: e01=(w,0,0), e02=(w,0,h)
  cross = (0·h - 0·0, 0·w - w·h, w·0 - 0·w) = (0, -wh, 0) → -Y ✓

Triangle [0,2,3]: e02=(w,0,h), e03=(0,0,h)
  cross = (0·h - h·0, h·0 - w·h, w·0 - 0·0) = (0, -wh, 0) → -Y ✓
```

### +X Face (case 2, line 380-389)

```
v0 = (bx, by,     bz)
v1 = (bx, by+w,   bz)
v2 = (bx, by+w,   bz+h)
v3 = (bx, by,     bz+h)

Triangle [0,1,2]: e01=(0,w,0), e02=(0,w,h)
  cross = (w·h - 0·w, 0·0 - 0·h, 0·w - w·0) = (wh, 0, 0) → +X ✓

Triangle [0,2,3]: e02=(0,w,h), e03=(0,0,h)
  cross = (w·h - h·0, h·0 - 0·h, 0·0 - w·0) = (wh, 0, 0) → +X ✓
```

### -X Face (case 3, line 391-400)

```
v0 = (bx, by,     bz)
v1 = (bx, by,     bz+h)
v2 = (bx, by+w,   bz+h)
v3 = (bx, by+w,   bz)

Triangle [0,1,2]: e01=(0,0,h), e02=(0,w,h)
  cross = (0·h - h·w, h·0 - 0·h, 0·w - 0·0) = (-wh, 0, 0) → -X ✓

Triangle [0,2,3]: e02=(0,w,h), e03=(0,w,0)
  cross = (w·0 - h·w, h·0 - 0·0, 0·w - w·0) = (-wh, 0, 0) → -X ✓
```

### +Z Face (case 4, line 402-411)

```
v0 = (bx,     by,     bz)
v1 = (bx+w,   by,     bz)
v2 = (bx+w,   by+h,   bz)
v3 = (bx,     by+h,   bz)

Triangle [0,1,2]: e01=(w,0,0), e02=(w,h,0)
  cross = (0·0 - 0·h, 0·w - w·0, w·h - 0·w) = (0, 0, wh) → +Z ✓

Triangle [0,2,3]: e02=(w,h,0), e03=(0,h,0)
  cross = (h·0 - 0·h, 0·0 - w·0, w·h - h·0) = (0, 0, wh) → +Z ✓
```

### -Z Face (case 5/default, line 413-423)

```
v0 = (bx,     by,     bz)
v1 = (bx,     by+h,   bz)
v2 = (bx+w,   by+h,   bz)
v3 = (bx+w,   by,     bz)

Triangle [0,1,2]: e01=(0,h,0), e02=(w,h,0)
  cross = (h·0 - 0·h, 0·w - 0·0, 0·h - h·w) = (0, 0, -wh) → -Z ✓

Triangle [0,2,3]: e02=(w,h,0), e03=(w,0,0)
  cross = (h·0 - 0·0, 0·w - w·0, w·0 - h·w) = (0, 0, -wh) → -Z ✓
```

### Conclusion

All 12 triangles (2 per face × 6 faces) produce normals pointing in the correct outward direction. The winding order is consistent with `FrontFace::Ccw` and `CullMode::Back` in the render pipeline.

---

## Finding 5: Atomic Contention on draw_meta

**Severity:** Low
**Type:** Performance

### Description

Six workgroups per slot (one per face direction) compete on two atomic counters in `draw_meta`:
- `draw_meta[meta_base + 1]` — vertex count
- `draw_meta[meta_base + 3]` — index count

Each of the 62 active threads per workgroup performs `atomicAdd` for every merged quad it emits. With 6 workgroups × 62 threads = 372 threads, and each thread emitting multiple quads, the total atomic operations per chunk is:

```
total_atomics = 2 × total_quads_across_all_faces
```

For the test scene (1104 quads), this is 2208 atomic operations on 2 memory locations.

### Impact Analysis

Atomic operations on the same address serialize at the memory controller. Each `atomicAdd` takes ~50-100 cycles on modern GPUs. With 2208 operations serialized, the total atomic overhead is:

```
2208 × ~80 cycles = ~176,640 cycles ≈ 0.1ms at 1.5 GHz
```

This is small for a single chunk but scales linearly with chunk count and quad count. For 100 resident chunks averaging 5000 quads each, this becomes ~50ms of pure atomic stall — unacceptable.

### Recommended Fix

Use a workgroup-level accumulator in shared memory. Each thread within a workgroup accumulates its quad count locally:

```wgsl
var<workgroup> wg_vert_count: atomic<u32>;
var<workgroup> wg_idx_count: atomic<u32>;
var<workgroup> wg_vert_base: u32;
var<workgroup> wg_idx_base: u32;

// During merge: thread accumulates locally
let local_verts = atomicAdd(&wg_vert_count, 4u);

// After workgroupBarrier, thread 0 claims the global range:
if local_id.x == 0u {
    wg_vert_base = atomicAdd(&draw_meta[meta_base + 1u], atomicLoad(&wg_vert_count));
    wg_idx_base = atomicAdd(&draw_meta[meta_base + 3u], atomicLoad(&wg_idx_count));
}
workgroupBarrier();

// Threads write to: slot_vert_base + wg_vert_base + local_verts
```

This reduces 372 global atomics to 6 (one per workgroup). Workgroup-level atomics are much faster since they operate on shared memory (~5 cycles vs ~80).

**Complexity:** Requires a two-pass approach within each thread — first count quads, then barrier, then emit vertices with the resolved base offset. This is a significant refactor.

### Deferral Justification

For Phase 1 (1 chunk, 1104 quads), the overhead is unmeasurable. This becomes important at Phase 2+ when multiple chunks are resident.

---

## Finding 6: Private Memory Pressure

**Severity:** Low
**Type:** Performance

### Description

Each thread allocates `var<private> processed: array<u32, 121>` = 484 bytes in private address space. With 64 threads per workgroup (62 active), this is:

```
64 × 484 = 30,976 bytes per workgroup
```

### GPU Register File Context

Typical GPU register files:
- AMD RDNA 3: 256 VGPRs × 4 bytes = 1,024 bytes per thread
- NVIDIA Ada: 255 registers × 4 bytes = 1,020 bytes per thread

The 484-byte processed bitmap alone exceeds the register file capacity for one thread. The WGSL compiler will spill the entire array to "scratch" memory (device-local VRAM accessed via global memory instructions).

### Impact

Every `proc_get()` and `proc_set()` becomes a global memory load/store instead of a register access. For the merge loop, which checks the processed bitmap for every cell:

```
Minimum proc_get calls: 62 × 62 = 3,844 (initial scan)
Additional for width/height extension: ~2,000-5,000
proc_set calls: 62 × 62 = 3,844 (marking processed)

Total: ~10,000-12,000 private memory accesses per thread
```

At ~400 cycles per global memory access (with caching), this is:
```
~10,000 × ~100 cycles (cached) = ~1M cycles ≈ 0.6ms at 1.5 GHz per thread
```

With 62 active threads running in parallel (GPU hides latency via interleaving), the effective wall time is much less, but memory bandwidth is consumed.

### Recommended Fix (Future)

**Tiled processing:** Divide the 62×62 slice into 8×8 sub-tiles. Each sub-tile's processed bitmap is 64 bits = 2 u32 = 8 bytes, which fits in registers. Process sub-tiles sequentially, merging quads within each tile. Quads that span tile boundaries are split.

**Trade-off:** Tile boundaries produce more quads (worse merge quality) but dramatically reduce memory pressure. For voxel geometry, most quads are small enough to fit within 8×8 tiles.

### Deferral Justification

The current approach is correct and produces optimal merge quality. The performance cost is ~1ms per chunk on mid-range hardware, which is acceptable for Phase 1. Tiled processing is a Phase 5 optimization alongside meshlets.

---

## Finding 7: Material Handling — Hardcoded

**Severity:** Known / Incomplete Feature
**Type:** Correctness (future)

### Description

Line 198:
```wgsl
let mat_id = 1u;
```

All voxels are assigned material ID 1 regardless of the chunk's palette. The greedy merge does not check material continuity — adjacent voxels with different palette entries are merged into a single quad.

### Correct Behavior (per spec)

The greedy merge should only merge adjacent faces if they share the same material. The merge loop's width/height extension must include a material equality check:

```
if face_visible AND material_of(this_voxel) == material_of(start_voxel):
    extend
else:
    stop
```

The material of a voxel is determined by the chunk palette and a per-voxel palette index. The current occupancy atlas stores only 1-bit-per-voxel (occupied/empty). Per-voxel palette indices require either:
1. A separate buffer (palette index atlas, similar to occupancy atlas but multi-bit)
2. Inline in the occupancy data (requires format change)

### Impact

With the current single-material test scene (stone + blue + emissive all sharing material_id=1 in practice), this produces visually correct results. When multiple materials are loaded (Phase 2 OBJ loading), quads will incorrectly span material boundaries, producing visible artifacts at material transitions.

### Recommended Fix

Deferred to Phase 2. When per-voxel material indices are added, update both the WGSL shader and the CPU reference to:
1. Read the palette index for each voxel during the merge
2. Compare against the start voxel's palette index
3. Stop extending when materials differ

The CPU reference (`mesh_cpu.rs`) has the same hardcoded `material_id: 1` (line 228, 277, 324) and must be updated in parallel.

---

## Finding 8: Coordinate Mapping — Verified Correct

**Severity:** None (verified)
**Type:** Mathematical verification

### Method

Trace the mapping from `(slice, primary, secondary)` to `(padded_x, padded_z, y_bit)` for each face direction. Compare against the CPU reference's per-face merge functions.

### Y Faces (case 0, 1)

```
WGSL:    px = primary + 1,  pz = secondary + 1,  y_bit = slice
CPU:     px = start_x + 1,  pz = start_z + 1,    y_bit = slice_y
Mapping: primary=start_x,   secondary=start_z,    slice=slice_y
```
Width extends primary (X), height extends secondary (Z). **Matches.**

### X Faces (case 2, 3)

```
WGSL:    px = slice + 1,    pz = secondary + 1,  y_bit = primary
CPU:     px = slice_x + 1,  pz = start_z + 1,    y_bit = start_y
Mapping: primary=start_y,   secondary=start_z,    slice=slice_x
```
Width extends primary (Y), height extends secondary (Z). **Matches.**

### Z Faces (case 4, 5)

```
WGSL:    px = primary + 1,  pz = slice + 1,      y_bit = secondary
CPU:     px = start_x + 1,  pz = slice_z + 1,    y_bit = start_y
Mapping: primary=start_x,   secondary=start_y,    slice=slice_z
```
Width extends primary (X), height extends secondary (Y). **Matches.**

### Vertex Position Mapping

For +Y face (case 0), the base position is:
```
bx = world_off.x + f32(primary + 1)    // usable_x + padding
by = world_off.y + f32(slice + 1) + 1  // usable_y + padding + 1 (face offset)
bz = world_off.z + f32(secondary + 1)  // usable_z + padding
```

The `+ 1.0` on `by` for +Y and `bx` for +X and `bz` for +Z correctly offsets the face to the far side of the voxel (the face between this voxel and its empty neighbor).

For -Y, -X, -Z: no `+ 1.0` offset — the face sits at the near side of the voxel. **Correct.**

### Width/Height Extension Mapping

Width extends `primary` (first scan axis), height extends `secondary` (second scan axis). The vertex position uses `f32(width)` and `f32(height)` in the corresponding axes. The processed bitmap tracks `(primary, secondary)` coordinates.

**All mappings are consistent between the WGSL shader, CPU reference, and legacy mesher.**

---

## Finding 9: Face Culling — Verified Correct

**Severity:** None (verified)
**Type:** Mathematical verification

### Method

Compare the WGSL `cull_column` function against the legacy `cull.rs` implementation, accounting for the u64 → vec2u emulation.

### u64 Emulation Verification

The WGSL shader stores u64 values as `vec2u(lo, hi)` where `lo` contains bits [0..31] and `hi` contains bits [32..63].

**Right shift by 1:**
```wgsl
fn shr1(v: vec2u) -> vec2u {
    let lo = (v.x >> 1u) | (v.y << 31u);  // bit 32 carries into bit 31 of lo
    let hi = v.y >> 1u;
    return vec2u(lo, hi);
}
```
Equivalent to `u64 >> 1`. The carry from hi to lo via `v.y << 31` is correct. **Verified.**

**Left shift by 1:**
```wgsl
fn shl1(v: vec2u) -> vec2u {
    let lo = v.x << 1u;
    let hi = (v.y << 1u) | (v.x >> 31u);  // bit 31 of lo carries into bit 0 of hi
    return vec2u(lo, hi);
}
```
Equivalent to `u64 << 1`. The carry from lo to hi via `v.x >> 31` is correct. **Verified.**

### Culling Logic Verification

| Direction | Legacy (u64) | WGSL (vec2u) | Equivalent? |
|---|---|---|---|
| +Y | `col & !(col >> 1)` | `and2(col, not2(shr1(col)))` | Yes — `and2` = `&`, `not2` = `!`, `shr1` = `>> 1` |
| -Y | `col & !(col << 1)` | `and2(col, not2(shl1(col)))` | Yes |
| +X | `col & !neighbor_px` | `and2(col, not2(neighbor))` | Yes |
| -X | `col & !neighbor_nx` | `and2(col, not2(neighbor))` | Yes |
| +Z | `col & !neighbor_pz` | `and2(col, not2(neighbor))` | Yes |
| -Z | `col & !neighbor_nz` | `and2(col, not2(neighbor))` | Yes |

### Usable Mask Verification

Legacy: `(result >> 1) & ((1u64 << 62) - 1)` where `(1u64 << 62) - 1 = 0x3FFFFFFFFFFFFFFF`

WGSL `to_usable`:
```wgsl
let shifted = shr1(v);  // >> 1
return vec2u(shifted.x & 0xFFFFFFFF, shifted.y & 0x3FFFFFFF);
```

`0xFFFFFFFF` (lo) + `0x3FFFFFFF` (hi) = 62 bits set. Equivalent to `0x3FFFFFFFFFFFFFFF`. **Verified.**

### Neighbor Selection Verification

`get_neighbor` selects the adjacent column based on face direction:
- +X: `read_col(x + 1, z)` — correct, checks if voxel at x+1 is empty
- -X: `read_col(x - 1, z)` — correct
- +Z: `read_col(x, z + 1)` — correct
- -Z: `read_col(x, z - 1)` — correct
- Y faces: returns `(0, 0)` — correct, Y neighbors are within the same column (handled by shift)

**All culling logic is mathematically equivalent to the legacy implementation.**

---

## Summary Table

| # | Finding | Severity | Type | Action |
|---|---------|----------|------|--------|
| 1 | Stale index pattern comment | Low | Documentation | Fix now |
| 2 | Redundant cull recomputation | Medium | Performance | Fix at Phase 2 |
| 3 | Overflow guard leaks counters | Medium | Correctness | Fix now (clamp in build_indirect) |
| 4 | Vertex winding order | None | Verified correct | — |
| 5 | Atomic contention (6 WG × 62 threads) | Low | Performance | Fix at Phase 5 |
| 6 | Private memory pressure (484 B/thread) | Low | Performance | Fix at Phase 5 |
| 7 | Material hardcoded to 1 | Known | Incomplete | Fix at Phase 2 |
| 8 | Coordinate mapping | None | Verified correct | — |
| 9 | Face culling math | None | Verified correct | — |
