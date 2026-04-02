# Proposal: Bricklet-Morton Column Ordering for Chunk Occupancy Atlas

**Type:** spec
**Status:** proposed
**Date:** 2026-03-22
**Amends:** [chunk-occupancy-atlas](chunk-occupancy-atlas.md)

> Change the inter-column memory layout from row-major to bricklet-grouped Morton order. Within each 8×8 bricklet, columns are Morton-ordered for cache-coherent ray traversal. Across bricklets, ordering is row-major. The u64 Y-column representation is unchanged.

---

## Motivation

The occupancy atlas is the hottest data structure in the pipeline. It is read by:

- **R-1 mesh rebuild** — full scan of all 4096 columns per chunk
- **R-6 cascade build** — random-access per-column reads during DDA ray march (millions of rays per frame)
- **I-3 summary rebuild** — full scan grouped by bricklet
- **Picking / shadow / collision** — per-column reads during `traceFirstHit`

The current row-major layout (`column_index = x * 64 + z`) is optimal for R-1's slice-by-slice iteration but hostile to R-6's diagonal ray traversal. A ray moving diagonally through XZ space accesses columns that are stride-64 apart in memory — every step is a cache miss. GPU L1 cache lines are 128 bytes (16 columns × 8 bytes/column). Row-major diagonal access utilizes ~1/16 of each loaded cache line.

R-6 is the dominant consumer by access count (millions of column reads per frame vs R-1's thousands). Optimizing the layout for traversal coherence at a modest cost to meshing is the right tradeoff.

---

## Proposed Layout: Bricklet-Grouped Morton

### Addressing Formula

```
bricklet_x     = x >> 3                            // which bricklet column (0..7)
bricklet_z     = z >> 3                            // which bricklet row (0..7)
bricklet_index = bricklet_x * 8 + bricklet_z      // row-major across 64 bricklets

local_x        = x & 7                             // position within bricklet (0..7)
local_z        = z & 7
local_index    = morton_encode_2d(local_x, local_z) // Morton within 8×8 (0..63)

column_index   = bricklet_index * 64 + local_index  // final offset into atlas
```

The u64 Y-column at each index is unchanged — 64 bits of Y occupancy, accessed as two u32 words:

```
u32_index  = column_index * 2 + (y >> 5)
bit_within = y & 31
is_occupied = (atlas[slot_offset + u32_index] >> bit_within) & 1
```

### Morton Encode (6-bit domain, WGSL)

For the 3-bit local coordinates (0..7), the Morton encode is trivial:

```wgsl
fn morton_encode_2d_3bit(x: u32, z: u32) -> u32 {
    // Spread 3 bits: abc -> a0b0c0
    var a = x & 7u;
    a = (a | (a << 2u)) & 0x09u;  // 0b001001
    a = (a | (a << 1u)) & 0x15u;  // 0b010101

    var b = z & 7u;
    b = (b | (b << 2u)) & 0x09u;
    b = (b | (b << 1u)) & 0x15u;

    return a | (b << 1u);
}
```

**Cost:** 4 shifts + 4 ANDs + 2 ORs = **10 integer ALU ops.** Negligible on GPU — integer ALU is essentially free relative to the memory latency this optimization targets.

### Morton Decode (inverse)

```wgsl
fn morton_decode_2d_3bit(code: u32) -> vec2u {
    var x = code & 0x15u;         // 0b010101
    x = (x | (x >> 1u)) & 0x09u; // 0b001001
    x = (x | (x >> 2u)) & 0x07u; // 0b000111

    var z = (code >> 1u) & 0x15u;
    z = (z | (z >> 1u)) & 0x09u;
    z = (z | (z >> 2u)) & 0x07u;

    return vec2u(x, z);
}
```

---

## Cache Analysis

### GPU Cache Parameters (typical discrete GPU)

| Parameter | Value |
|---|---|
| L1 cache line | 128 bytes |
| Columns per cache line | 16 (128 B ÷ 8 B/column) |
| L1 per SM | 48–128 KB |
| L2 total | 2–6 MB |
| Occupancy atlas per slot | 8 KB (4096 columns × 2 u32) |

### Current Layout (Row-Major)

| Ray direction | Stride between consecutive steps | Cache lines per 8 steps | Utilization |
|---|---|---|---|
| +Z (constant X) | 1 column (8 B) | 1 line | **100%** — sequential |
| +X (constant Z) | 64 columns (512 B) | 8 lines | **12.5%** — one useful column per line |
| Diagonal (45°) | 65 columns (520 B) | 8 lines | **12.5%** |
| Shallow angle | Variable, typically 1–64 | 1–8 lines | **12–100%** |

### Proposed Layout (Bricklet Morton)

Within each 8×8 bricklet (64 columns = 512 bytes = 4 cache lines):

| Ray direction | Cache lines touched within bricklet | Utilization |
|---|---|---|
| +Z (constant X) | 2 lines (Morton stride-2 within tile) | **50%** — slight degradation from 100% |
| +X (constant Z) | 2 lines | **50%** — major improvement from 12.5% |
| Diagonal (45°) | 2–3 lines | **33–50%** — major improvement from 12.5% |
| Any direction | 1–4 lines (all within same 512 B block) | **25–100%** |

**Net effect:** Diagonal and X-axis rays improve 4–8× in cache utilization. Z-axis rays degrade 2× within a bricklet but this is offset by the bricklet's small footprint (4 cache lines total — likely all resident in L1 after first access).

### Cross-Bricklet Behavior

When a ray exits one bricklet and enters the adjacent one, the new bricklet's 512 bytes must be loaded. In row-major, this happens every 8 Z-steps or every 1 X-step. In bricklet Morton, this happens every ~8 steps in any direction — more uniform, more predictable for the cache prefetcher.

---

## Impact on Each Pipeline Stage

### R-6 Cascade Build (primary beneficiary)

Each probe traces rays through the occupancy atlas via DDA. A typical cascade 0 ray traverses 1–4 columns. Cascade 3 traverses up to ~32 columns. Most rays are non-axis-aligned (probe directions are octahedrally distributed).

**Before:** Non-axis-aligned rays cause ~1 cache miss per DDA step (stride-64 or stride-65 between columns).

**After:** Rays within a bricklet reuse 2–4 cache lines loaded on the first access. Cross-bricklet transitions (~every 8 steps) load a new 4-line block. Expected reduction in L1 misses: **3–6× for typical probe rays**.

### R-1 Mesh Rebuild (slight overhead)

The greedy mesher iterates slices row-by-row: for each Y level, it reads columns at (x, z) and (x+1, z) for face culling. In the current layout, (x, z) and (x+1, z) are stride-64 apart. In bricklet Morton, columns within the same bricklet are nearby; at bricklet boundaries (every 8 columns along X), the mesher crosses to a new contiguous block.

**Impact:** The mesher already has poor cache behavior for X-axis adjacency in row-major (stride-64). Bricklet Morton makes intra-bricklet adjacency better and inter-bricklet adjacency the same. Net: **neutral to slightly positive**.

The mesher needs a `column_at(x, z)` helper that encodes the bricklet Morton index. This adds ~10 ALU ops per column access. For 4096 columns per slice × ~4 slices = ~16K column reads per mesh rebuild, the overhead is ~160K integer ops — negligible on GPU.

### I-3 Summary Rebuild (naturally aligned)

I-3 computes one summary bit per bricklet by scanning all 64 columns in the bricklet. In bricklet Morton layout, those 64 columns are contiguous in memory — a single sequential scan of 512 bytes. **This is identical to the current layout** (where the 64 columns of a bricklet are also accessed together, just at different offsets).

The bricklet-grouped layout actually makes I-3 slightly simpler: iterate bricklet_index 0..63, for each read columns [bricklet_index * 64 .. bricklet_index * 64 + 63] sequentially.

### traceFirstHit / traceSegments (primary beneficiary alongside R-6)

Same DDA inner loop as R-6. Same cache benefit. Picking rays, shadow rays, and collision queries all benefit from the same improved spatial locality.

---

## Impact on Existing Invariants

### OCC-1 (addressing) — CHANGES

Current: `column_index = x * 64 + z`
Proposed: `column_index = bricklet_index * 64 + morton_encode_2d(x & 7, z & 7)` where `bricklet_index = (x >> 3) * 8 + (z >> 3)`

The bit-level Y addressing within a column is unchanged: `u32_index = column_index * 2 + (y >> 5)`, `bit_within = y & 31`.

### OCC-2 through OCC-6 — UNCHANGED

Padding, inner/boundary classification, version monotonicity, slot isolation — all operate at the (x, y, z) voxel level, not at the column storage level. The column_index formula is an implementation detail that doesn't affect these invariants.

### SUM-1 through SUM-3 — UNCHANGED

Bricklet summary bits are defined by (bx, by, bz) bricklet coordinates. The scan to compute them reads all columns in a bricklet regardless of their memory order. With bricklet-grouped storage, the scan is contiguous.

---

## Migration Path

### Phase 1 (demo): Row-major

The demo scene is small (~200 chunks). Cache pressure is minimal. Implement the row-major layout as currently specified. This gets pixels on screen fastest.

### Phase 2 (real meshes): Evaluate

Profile R-6 cascade build with real scenes. If L1 cache miss rate in the DDA inner loop is >30%, implement bricklet Morton. If not, defer.

### Phase 3 (if adopted): Bricklet Morton

1. Change `column_index` formula in `chunk-occupancy-atlas.md`
2. Add `morton_encode_2d_3bit` / `morton_decode_2d_3bit` utility functions to `dda.wgsl`
3. Update I-2 upload to write columns in bricklet Morton order
4. Update R-1 mesh rebuild to decode via `column_at(x, z)` helper
5. Update I-3 to iterate bricklets contiguously (natural with this layout)
6. Update all tests: column addressing roundtrip must use new formula

The change is localized — only the column_index formula changes. All u64 Y-column operations, all invariants except OCC-1's formula, and all higher-level stage contracts are unaffected.

---

## What This Does NOT Change

- The u64 Y-column representation (64 bits of Y per column)
- The chunk size (64³ padded, 62³ usable)
- The 1-voxel padding convention
- The bricklet grid dimensions (8×8×8 = 512 bricklets)
- The occupancy_summary addressing (bricklet-indexed, independent of column order)
- Any stage's preconditions, postconditions, or buffer contracts
- The `traceSegments` / `traceFirstHit` API contract

---

## Constants

```
BRICKLET_DIM     = 8       // columns per bricklet axis
BRICKLETS_PER_AXIS = 8     // 64 / 8
COLUMNS_PER_BRICKLET = 64  // 8 × 8
BRICKLETS_PER_CHUNK = 64   // 8 × 8 (in XZ plane; Y is within the column)
BYTES_PER_BRICKLET = 512   // 64 columns × 8 bytes/column
CACHE_LINES_PER_BRICKLET = 4  // 512 / 128
```

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Addressing roundtrip:** For all (x, z) ∈ [0, 63]², verify `decode(encode(x, z)) == (x, z)`.
2. **Morton encode correctness:** Verify `morton_encode_2d_3bit` produces the Z-curve ordering for all 64 values.
3. **Column isolation:** Writing to bricklet-Morton column_index for (x, z) does not modify any other (x', z').
4. **Bricklet contiguity:** For each bricklet, verify its 64 columns occupy indices [bricklet_index * 64 .. bricklet_index * 64 + 63].

### Property tests (Rust, randomized)

5. **Spatial locality:** For 1000 random (x, z) pairs where |x1-x2| ≤ 1 and |z1-z2| ≤ 1, verify that the Morton column indices are within 4 of each other (same or adjacent cache line).
6. **Equivalence:** For 1000 random occupancy patterns, verify that reading all 64³ voxels produces the same result regardless of row-major vs bricklet-Morton storage — the data is the same, only the addressing changes.

### GPU validation

7. **R-6 cache comparison:** Run cascade build on the same scene with row-major and bricklet-Morton layouts. Verify identical visual output. Measure L1 miss rate if hardware counters are available.
8. **DDA hit agreement:** For 10,000 random rays, verify `traceFirstHit` returns the same result with both layouts.

---

## See Also

- [chunk-occupancy-atlas](chunk-occupancy-atlas.md) — current spec (row-major, OCC-1)
- [traversal-acceleration](../traversal-acceleration.md) — DDA design, column-aware inner loop
- [occupancy-summary](occupancy-summary.md) — bricklet grid (naturally aligned with this proposal)
- [R-6 cascade build](../stages/R-6-cascade-build.md) — primary beneficiary of improved cache coherence
- [R-1 mesh rebuild](../stages/R-1-mesh-rebuild.md) — slight addressing overhead, neutral net impact

### Sources

- [Nocentino & Rhodes — Optimizing memory access on GPUs using Morton order indexing (2010)](https://www.nocentino.com/Nocentino10.pdf)
- [Fabian Giesen — Texture tiling and swizzling](https://fgiesen.wordpress.com/2011/01/17/texture-tiling-and-swizzling/)
- [Jeroen Baert — Morton encoding/decoding implementations](https://www.forceflow.be/2013/10/07/morton-encodingdecoding-through-bit-interleaving-implementations/)
- [Computing Morton Codes with a WebGPU Compute Shader](https://lvngd.com/blog/computing-morton-codes-webgpu-compute-shader/)
- [Analyzing block locality in Morton-order and Morton-hybrid matrices (ACM)](https://dl.acm.org/doi/10.1145/1327312.1327315)
