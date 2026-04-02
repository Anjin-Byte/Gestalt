# Variable Mesh Pool Allocation

**Type:** spec
**Status:** current
**Date:** 2026-04-01
**Depends on:** [gpu-chunk-pool](gpu-chunk-pool.md), [R-1-mesh-rebuild](stages/R-1-mesh-rebuild.md)

> Replaces the fixed per-slot mesh allocation (Option A) with a GPU-driven variable-size allocator (Option B) for vertex and index pools. Eliminates the 4096-quad-per-chunk ceiling that causes face loss on complex geometry. All allocation is GPU-side — zero CPU readback, zero sync points.

---

## Problem

The current mesh pool reserves fixed space per slot:
- 16384 vertices × 16 bytes = 256 KB per slot
- 24576 indices × 4 bytes = 96 KB per slot
- 1024 slots = 256 MB vertices + 96 MB indices = 352 MB total

A complex curved surface (organic mesh, bust, terrain) can produce 10,000–30,000 visible faces per chunk after face culling. The greedy merge can only collapse coplanar same-material faces — on curved surfaces, most faces are unique. At >4096 quads per chunk, the fixed allocation silently drops geometry. The GPU shader's F3 overflow guard discards quads via a race condition on atomic counters, causing arbitrary face directions to disappear depending on GPU thread scheduling.

This is not a future concern — it blocks Phase 2 OBJ loading for any non-trivial mesh.

---

## Design Principle

The GPU drives itself. The allocation pipeline runs entirely on the GPU within a single command encoder submission. No CPU readback. No sync points. No CPU-GPU round trips. The CPU's role is limited to configuring the pool budget at init time and resetting the allocator when a new scene is loaded.

---

## Architecture: Three-Pass GPU Pipeline

The mesh rebuild pipeline becomes three sequential compute passes within a single command encoder, replacing the current single-pass R-1:

```
Pass 1 — Count
  Input:  occupancy atlas, palette, index_buf, palette_meta
  Output: per-slot quad count (u32 per slot)
  Method: same face-cull + greedy-merge algorithm, but ONLY counts — does not emit vertices

Pass 2 — Prefix Sum (Allocate)
  Input:  per-slot quad counts
  Output: per-slot offsets (vertex_offset, index_offset) in the shared pool
  Method: exclusive prefix sum over quad counts × 4 (verts) and × 6 (indices)

Pass 3 — Write
  Input:  occupancy atlas, palette, offsets from pass 2
  Output: vertex_pool, index_pool (at allocated offsets)
  Method: same face-cull + greedy-merge algorithm, emits vertices at the computed offsets
```

### Why Three Passes

The current single-pass shader uses `atomicAdd` to claim vertex/index slots from a per-chunk counter. With variable allocation, the write offset depends on the TOTAL count from ALL preceding chunks — information that isn't available until all chunks have been counted. The prefix sum bridge between counting and writing is the standard GPU pattern for this (used in GPU particle systems, GPU sort, GPU compaction).

### Why Not Two Passes (Count + Write)

A two-pass approach (count on GPU, allocate on CPU, write on GPU) requires a GPU→CPU readback between passes. This introduces a sync point — the CPU must wait for the count pass to complete, read back the counts, compute offsets, upload them, then dispatch the write pass. This violates the GPU-drives-itself principle and adds per-frame latency.

The prefix sum pass runs entirely on the GPU and replaces the CPU allocation step.

---

## Pass 1: Count

A compute shader structurally identical to the current `mesh_rebuild.wgsl`, but instead of emitting vertices, it increments a per-slot quad counter.

### Dispatch

Same as current R-1: `(slot_count, 6, 1)` with `@workgroup_size(64, 1, 1)`.

### Output

```
mesh_counts[slot]: u32  — total quad count for this slot (across all 6 face directions)
```

Written via `atomicAdd` by 372 threads per slot (6 faces × 62 slices), same contention pattern as current R-1. The counter is per-slot, not global, so contention is bounded.

### What Changes from Current R-1

- Remove all vertex/index emission code (write_vertex, index_pool writes)
- Remove the F3 overflow guard (no overflow — just counting)
- Keep the face cull + visibility bitmap + greedy merge logic identically
- Each merged quad increments `atomicAdd(&mesh_counts[slot], 1u)` instead of claiming vertices

### Per-Slot vs Per-Direction Counting

Counting per-slot (one counter for all 6 face directions) is sufficient for allocation. Per-direction counting would enable direction-level overflow detection but adds complexity for no allocation benefit.

---

## Pass 2: Prefix Sum

Computes exclusive prefix sum over `mesh_counts[]` to produce per-slot offsets.

### Algorithm

For N slots, each with quad count `q[i]`:

```
vertex_offset[0] = 0
vertex_offset[i] = vertex_offset[i-1] + q[i-1] * 4    // 4 vertices per quad

index_offset[0] = 0
index_offset[i] = index_offset[i-1] + q[i-1] * 6      // 6 indices per quad
```

The last element's offset + count gives the total pool usage, which can be checked against the budget.

### Implementation

For ≤1024 slots, a single-workgroup prefix sum is sufficient:

```wgsl
@compute @workgroup_size(256, 1, 1)
fn prefix_sum(@builtin(local_invocation_id) lid: vec3u) {
    // Load quad counts into shared memory
    // Blelloch exclusive scan (up-sweep + down-sweep)
    // Write vertex_offset = prefix * 4, index_offset = prefix * 6
}
```

With 256 threads processing 1024 elements (4 per thread), this completes in O(log N) steps within a single workgroup. No multi-workgroup coordination needed.

### Output

```
mesh_offset_table[slot]: vec4u
  .x = vertex_offset  (in vertices)
  .y = vertex_count   (= quad_count * 4)
  .z = index_offset    (in indices)
  .w = index_count     (= quad_count * 6)
```

Also writes a single `total_usage` word for optional overflow detection:
```
mesh_total[0] = total_vertices  (last offset + last count)
mesh_total[1] = total_indices
```

### Budget Overflow

If `total_vertices > VERTEX_POOL_CAPACITY` or `total_indices > INDEX_POOL_CAPACITY`, the scene doesn't fit. Options:
- Clamp the last N slots to zero (drop furthest chunks)
- Signal overflow to CPU via a flag buffer (CPU reads next frame, logs warning)
- For Phase 2: just log it and accept the clamp

---

## Pass 3: Write

Structurally identical to the current `mesh_rebuild.wgsl`, but reads its write offsets from `mesh_offset_table` instead of computing `slot * MAX`.

### Key Differences from Current R-1

```wgsl
// OLD (fixed allocation):
let slot_vert_base = slot * MAX_VERTS_PER_CHUNK * VERTEX_STRIDE;
let slot_idx_base = slot * MAX_INDICES_PER_CHUNK;
let meta_base = slot * DRAW_META_STRIDE;

// NEW (variable allocation):
let alloc = mesh_offset_table[slot];
let slot_vert_base = alloc.x * VERTEX_STRIDE;   // vertex_offset × 4 words per vertex
let slot_idx_base = alloc.z;                      // index_offset (in indices)
let max_quads = alloc.y / 4u;                     // allocated vertex count / 4
```

The overflow guard (F3) checks against the allocated count instead of the global max:
```wgsl
if vert_claim + 4u > alloc.y || idx_claim + 6u > alloc.w {
    continue;  // should not happen if count pass was correct
}
```

In the ideal case, the count pass and write pass produce identical quad counts, so the overflow guard never fires. It exists as a safety net for floating-point or race condition discrepancies.

### Dispatch

Same as Pass 1: `(slot_count, 6, 1)` with `@workgroup_size(64, 1, 1)`.

---

## Changes to build_indirect.wgsl

Replace the fixed-stride computation:

```wgsl
// OLD:
indirect_buf[ind_base + 2u] = slot * MAX_INDICES_PER_CHUNK;
indirect_buf[ind_base + 3u] = slot * MAX_VERTS_PER_CHUNK;

// NEW:
let alloc = mesh_offset_table[slot];
indirect_buf[ind_base]      = alloc.w;     // index_count
indirect_buf[ind_base + 1u] = select(0u, 1u, alloc.w > 0u && vis != 0u);
indirect_buf[ind_base + 2u] = alloc.z;     // first_index = index_offset
indirect_buf[ind_base + 3u] = alloc.x;     // base_vertex = vertex_offset
indirect_buf[ind_base + 4u] = 0u;
```

`draw_meta` is no longer needed for indirect args — the offset table replaces it. The `draw_meta` buffer can be repurposed for diagnostics or removed.

---

## Changes to Vertex Shader

None. The vertex shader reads `vertex_pool[vi * 4]` where `vi = @builtin(vertex_index)`. The vertex_index already includes `base_vertex` from the indirect draw args. The shader is allocation-agnostic.

---

## Pool Sizing

The pool is a single large buffer shared by all slots:

```
MESH_VERTEX_POOL_CAPACITY = 4,194,304 vertices  (4M × 16 B = 64 MB)
MESH_INDEX_POOL_CAPACITY  = 6,291,456 indices    (6M × 4 B  = 24 MB)
Total: 88 MB
```

This supports ~4M total vertices across all chunks. For comparison:
- 100 chunks at 40K vertices each = 4M (fits)
- 1000 chunks at 4K vertices each = 4M (fits)
- 17 chunks at 60K vertices each = 1.02M (fits easily)

The budget is a compile-time constant, configurable for different GPU tiers.

### Memory Comparison

| Configuration | Vertex pool | Index pool | Total mesh |
|---|---|---|---|
| Current (Option A, 1024 slots) | 256 MB | 96 MB | 352 MB |
| Variable (64 MB budget) | 64 MB | 24 MB | 88 MB |

4× less memory, no per-chunk ceiling.

---

## New GPU Buffers

| Buffer | Size | Usage | Written by | Read by |
|---|---|---|---|---|
| `mesh_counts` | 4 B × MAX_SLOTS | STORAGE | Pass 1 (atomic) | Pass 2 |
| `mesh_offset_table` | 16 B × MAX_SLOTS | STORAGE | Pass 2 | Pass 3, build_indirect |
| `mesh_total` | 8 B | STORAGE | Pass 2 | (diagnostics) |

The existing `draw_meta` buffer is replaced by `mesh_offset_table` for indirect draw arg generation. `draw_meta` may be retained for backward compatibility with wireframe build but is no longer on the critical path.

---

## Implementation Steps

### Step 1 — Pool constants + allocator reset

Replace fixed per-slot constants with pool-wide budget constants. Add `mesh_counts`, `mesh_offset_table`, `mesh_total` buffers to `pool_gpu.rs`.

### Step 2 — Count shader (mesh_count.wgsl)

Fork `mesh_rebuild.wgsl`. Remove vertex/index emission. Replace with `atomicAdd(&mesh_counts[slot], 1u)` per merged quad. Keep face cull + visibility bitmap + greedy merge + material boundary logic identical.

### Step 3 — Prefix sum shader (prefix_sum.wgsl)

Single-workgroup Blelloch scan over `mesh_counts`. Writes `mesh_offset_table` with per-slot vertex_offset, vertex_count, index_offset, index_count.

### Step 4 — Update mesh_rebuild.wgsl (write pass)

Read offsets from `mesh_offset_table`. Replace `slot * MAX` with `alloc.x / alloc.z`. Update overflow guard to check allocated count.

### Step 5 — Update build_indirect.wgsl

Read from `mesh_offset_table` instead of fixed strides. Remove `MAX_VERTS_PER_CHUNK` / `MAX_INDICES_PER_CHUNK` constants.

### Step 6 — Update lib.rs dispatch

Replace single R-1 dispatch with three-pass sequence: count → prefix_sum → write. All in one command encoder.

### Step 7 — Pool buffer sizing

vertex_pool and index_pool sized by budget constants instead of per-slot × MAX_SLOTS.

---

## What This Does Not Change

- Occupancy atlas: still fixed 32 KB per slot
- Palette, index_buf, palette_meta: still fixed per slot
- Chunk slot allocation: still CPU-managed
- I-3 summary rebuild: unchanged
- Vertex format: unchanged (16 bytes)
- Index format: unchanged (u32)
- Render pipelines: unchanged
- Greedy merge algorithm: unchanged (same face cull, same merge, same material boundaries)

---

## See Also

- [gpu-chunk-pool](gpu-chunk-pool.md) — Option A vs Option B discussion
- [R-1-mesh-rebuild](stages/R-1-mesh-rebuild.md) — mesh rebuild stage spec
- [material-aware-merge](material-aware-merge.md) — greedy merge design (unchanged)
- [pipeline-stages](pipeline-stages.md) — where mesh pools are consumed
