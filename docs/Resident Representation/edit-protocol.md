# Edit Protocol

**Type:** spec
**Status:** current
**Date:** 2026-03-21

How voxel edits flow through the GPU-resident runtime without making derived structures haunted.

The GPU world is authoritative for **contents**. The CPU retains authority for **orchestration**.
That distinction is the whole trick.

---

## The Core Insight

Once voxels live on GPU, the problem is no longer "where are my voxels?"

The problem becomes: **how do I track which regions changed, which derived structures are stale, and which work must be redone — without dragging the whole world back to the CPU?**

Answer: a GPU-resident change pipeline with four distinct responsibilities.

---

## Four Responsibilities

Do not treat bookkeeping as one monolithic thing. Split it explicitly:

| Responsibility | Question it answers |
|---|---|
| **1. Authoritative voxel data** | What is the world? |
| **2. Change detection** | What just changed? |
| **3. Staleness propagation** | What derived things are now wrong? |
| **4. Work scheduling** | What work needs to happen, in what order, within what budget? |

Each is a separate concern. Conflating them is how engines become haunted.

---

## The Database Analogy

This architecture is isomorphic to a database with incremental maintenance:

| Database concept | Engine equivalent |
|---|---|
| Authoritative tables | Chunk occupancy, materials, versions |
| Invalidation indices | Dirty bits, boundary masks, stale product flags |
| Materialized views | Mesh, summaries, GI caches, visibility data |
| Triggers | Edit kernels marking dirty bits after writes |
| Background jobs | Rebuild passes consuming work queues |

Edits update the tables. Triggers mark the views stale. Background jobs rebuild the views.

The golden rule: **derived data is never edited directly; it is invalidated and rebuilt from authoritative voxel truth.**

Surgical incremental patching of every downstream artifact after each edit is how engines become folklore.

---

## Responsibility 1 — Authoritative Voxel Data

The GPU-resident world truth:
- `chunk_occupancy` — per-voxel occupancy bits
- `chunk_materials` — palette + index buffer
- `chunk_version` — monotonic counter per chunk
- `chunk_state` — residency and lifecycle flags

All producers (brush edits, boolean ops, density evaluators, procedural generation, voxelizer output) write here and nowhere else. No producer gets a special path.

**After any write, two things must happen atomically:**
1. The occupancy/material data is updated
2. `chunk_version` is incremented

The version increment is the signal. Everything downstream watches versions, not raw data.

---

## Responsibility 2 — Change Detection

Every edit kernel, while writing occupancy/materials, also writes to a compact GPU-side dirty map.

**The language of change is coarse, not per-voxel chatty.**

A brush op does not say "I changed 18,421 voxels, please panic." It says "chunks A, B, C were modified; boundaries +X and -Y touched; subregions 3 and 7 changed."

### Dirty structures maintained per chunk:

```
chunk_version[slot]         u32     monotonic counter, incremented on every write
dirty_chunks[slot/32]       u32     one bit per chunk slot — was this chunk written?
dirty_subregions[slot]      u32     one bit per 8³ subregion within the chunk (8 subregions per axis → 8³=512 → 16 words)
boundary_touch_mask[slot]   u32     6 bits: which faces (-X +X -Y +Y -Z +Z) were touched
```

The subregion granularity (8³ inside 64³) is the key design choice — see Granularity section below.

### What an edit kernel does:

```wgsl
// Inside edit compute shader, after writing occupancy
atomicAdd(&chunk_version[slot], 1u);
atomicOr(&dirty_chunks[slot >> 5], 1u << (slot & 31));
atomicOr(&dirty_subregions[slot * 16 + subregion_word], subregion_bit);
atomicOr(&boundary_touch_mask[slot], face_bits);  // if write was near a chunk face
```

This costs ~4 atomic writes per affected chunk per edit dispatch. That is the entire bookkeeping cost of an edit.

---

## Responsibility 3 — Staleness Propagation

A chunk change stales more than the chunk itself. This is the part most people forget.

### What a chunk change invalidates:

**Immediate (same chunk):**
- Greedy mesh
- Occupancy summaries (`occupancy_summary`, `chunk_flags`, `aabb`)
- Material palette statistics

**Second-order (triggered by mesh rebuild, not directly by occupancy change):**
- Meshlet cluster data — `stale_meshlet[slot]` is set by the mesh rebuild pass after writing a new mesh, not by the propagation pass. Meshlet staleness is a consequence of mesh staleness, not a direct consequence of voxel edits.

**Neighbor (adjacent chunks, only if boundary was touched):**
- Adjacent chunk greedy mesh (face visibility depends on neighbor occupancy)
- Adjacent chunk summaries if cross-boundary occupancy changed

**Regional (surrounding area, up to some radius):**
- GI / probe caches
- Radiance cascade intervals that passed through this region
- Shadow caches
- Any clipmap or LOD representation covering this chunk

### Propagation pass:

A lightweight GPU compute pass runs after edit kernels, before any rebuild work:

```
Input:  dirty_chunks, boundary_touch_mask
Output: stale_mesh_bitset, stale_summary_bitset, stale_lighting_bitset
        + neighbor dirty bits injected into dirty_chunks for adjacent slots
```

For each dirty chunk:
1. Set `stale_mesh_bitset[slot]`
2. Set `stale_summary_bitset[slot]`
3. If `boundary_touch_mask[slot]` has any face bits set:
   - Look up adjacent slot for each touched face
   - Set `stale_mesh_bitset[adjacent_slot]`
   - Set dirty bit for adjacent slot in `dirty_chunks`
4. Mark regional stale products (GI, lighting) within a configurable radius

This pass is cheap — one thread per dirty chunk. The dirty chunk count is typically small per frame for interactive edits.

---

## Responsibility 4 — Work Scheduling

### The Invariant

No matter how much authority shifts to the GPU over time, this must hold:

> **Scheduling metadata must be derived from authoritative GPU world state, not maintained as a second fragile truth elsewhere.**

The dirty bits, stale flags, and work queues are derived from chunk versions and occupancy — not from a parallel CPU bookkeeping system that must be kept in sync. That is what keeps the design coherent as the balance of authority shifts.

### The Migration Ladder

GPU-driven orchestration is not a binary switch. It is a ladder. Each stage is a valid stopping point, and each stage moves authority inward without requiring a redesign.

**Stage 1 — CPU tracks dirty chunks, GPU executes rebuilds**
- CPU scans world state, decides what is dirty, submits rebuild work explicitly
- GPU only executes what CPU tells it to
- Status: current Rust ChunkManager (CPU dirty tracking, WASM rebuild)

**Stage 2 — GPU marks dirty chunks, CPU reads queue counts and kicks passes**
- Edit kernels write dirty bits on GPU
- CPU reads queue counts (async, small counter readback — bytes, not world data)
- CPU uses counts to decide budget and submits dispatch calls
- GPU still waits for CPU to kick each pass
- Status: the primary target described in this document

**Stage 3 — GPU compacts worklists and prepares indirect dispatch args, CPU only provides frame budgets as uniforms**
- GPU compaction pass produces ready-to-dispatch indirect args
- CPU writes a single `frame_budget` uniform (max rebuilds this frame)
- CPU calls `dispatchWorkgroupsIndirect` — no world data ever read by CPU
- Status: the clean endpoint; GPU is fully self-scheduling within CPU-supplied budget

**Stage 4 — GPU chains dispatches itself with minimal CPU involvement**
- Persistent GPU threads or GPU-side dispatch chains handle rebuild scheduling autonomously
- CPU provides coarse policy (budget, priority weights) as infrequently updated uniforms
- WebGPU does not yet support persistent kernels, so this stage is deferred

### Current target: Stage 2 → Stage 3

The buffer set in this document supports both. Stage 2 reads `queue_counts` on CPU. Stage 3 eliminates that readback — the CPU never touches queue data, only provides a budget uniform and calls `dispatchWorkgroupsIndirect`.

The transition from Stage 2 to Stage 3 does not change the GPU buffer layout. It only changes which side decides how much work to dispatch per pass. That is a small code change, not an architectural redesign.

### Compacted work queues (GPU buffers):

```
mesh_rebuild_queue[]      array<u32>  chunk slots needing mesh rebuild, compacted
summary_rebuild_queue[]   array<u32>  chunk slots needing summary rebuild
lighting_update_queue[]   array<u32>  chunk slots needing GI/probe cache invalidation
queue_counts              array<u32>  atomic counters for each queue
```

Queues are populated by a compaction pass over `stale_*_bitset` arrays. Compaction uses GPU prefix sum or stream compaction (a single compute dispatch).

---

## The Full Edit Flow

```
Frame with edits:

1. Edit kernels dispatch
   ├─ write chunk_occupancy_atlas / chunk_palette_buf / chunk_index_buf
   ├─ atomicAdd chunk_version
   ├─ atomicOr dirty_chunks
   ├─ atomicOr dirty_subregions
   └─ atomicOr boundary_touch_mask

2. Propagation pass (one dispatch)
   ├─ read dirty_chunks + boundary_touch_mask
   ├─ expand neighbor dirty bits
   └─ write stale_mesh_bitset, stale_summary_bitset, stale_lighting_bitset

3. Compaction pass (one dispatch)
   ├─ compact stale bitsets → work queues
   └─ write queue_counts

4. CPU reads queue_counts (async, previous frame's data)
   └─ decides budget: N mesh rebuilds, M summary rebuilds this frame

5. Rebuild passes (budgeted)
   ├─ consume mesh_rebuild_queue[0..N]
   │   → write vertex/index pool
   │   → tag with built_from_version
   │   → set ready_to_swap[slot] bit
   │   → set stale_meshlet[slot] bit (triggers meshlet rebuild on next compaction)
   ├─ consume summary_rebuild_queue[0..M]
   │   → write chunk_flags, occupancy_summary, aabb
   │   → tag with built_from_version
   │   └─ set ready_to_swap[slot] bit
   └─ consume meshlet_rebuild_queue[0..P]
       → write meshlet_desc_pool, meshlet_index_pool
       → update meshlet_range_table[slot]
       → tag with built_from_version
       └─ set ready_to_swap_meshlet[slot] bit

6. Swap pass
   ├─ for each slot where ready_to_swap bit is set:
   │    verify built_from_version == chunk_version
   │      if match:   make live, clear ready_to_swap bit
   │      if mismatch: discard, clear ready_to_swap bit, re-queue
   └─ for each slot where ready_to_swap_meshlet bit is set:
        verify built_from_version == chunk_version
          if match:   update meshlet_range_table[slot], clear ready_to_swap_meshlet bit
          if mismatch: discard, clear ready_to_swap_meshlet bit, re-queue
```

---

## Version Tagging of Derived Products

Every derived artifact stores the version it was built from:

```
mesh_version[slot]              u32   chunk_version value when this mesh was built
summary_version[slot]           u32   chunk_version value when summaries were built
gi_cache_version[slot]          u32   chunk_version value when GI data was cached
```

On use or swap:
```
if mesh_version[slot] != chunk_version[slot]:
    artifact is stale — do not use
```

This invariant allows rebuilds to happen out of order, across multiple frames, without corruption. An edit that arrives while a rebuild is in flight just increments the version; the rebuild's result will fail the version check and be discarded. The chunk re-enters the rebuild queue.

This is the CPU `data_version` / `pending_mesh` pattern from the current Rust ChunkManager, promoted to GPU-resident and generalized across all derived product types.

---

## Granularity Design

**Too fine (per-voxel dirty tracking):**
- Atomics explode under concurrent brush strokes
- Queue sizes become enormous
- Bookkeeping cost approaches the cost of the work itself

**Too coarse (whole-chunk dirty only):**
- A single-voxel brush stroke remeshes entire chunks
- Lighting invalidation covers unnecessarily large regions
- Wasted rebuild work for large empty chunks

**Sweet spot: chunk-level truth + subregion-level change detection**

| Level | Granularity | Used for |
|---|---|---|
| Chunk | 64³ voxels | Mesh rebuild, residency, version tracking |
| Subregion | 8³ voxels (512 per chunk) | Fine-grained stale marking, lighting radius estimation |
| Boundary face | 6 bits per chunk | Neighbor mesh invalidation |
| Voxel | 1 bit | Actual truth — but never bookkept individually |

The subregion granularity rhymes with the brick intuition from the voxelizer without reusing brick CSR as runtime truth. Bricks are a producer-side acceleration structure. Subregions are a runtime bookkeeping granularity. They happen to be the same size because that size is natural for the data.

---

## GPU Buffer Set

Complete table of all GPU buffers in the edit protocol.

### Authoritative (Layer 1)

| Buffer | Format | Size | Description |
|---|---|---|---|
| `chunk_occupancy_atlas` | `array<u32>` per slot | 2048 u32 × N slots | Bitpacked voxel occupancy |
| `chunk_palette_buf` | `array<u32>` per slot | variable × N slots | Palette material IDs |
| `chunk_index_buf` | `array<u32>` per slot | variable × N slots | Bitpacked per-voxel palette indices |
| `chunk_version` | `array<u32>` | N slots | Monotonic version per chunk |
| `chunk_coord` | `array<vec4i>` | N slots × 16 bytes | Chunk world coordinate (xyz + padding) |

### Residency State (CPU-managed, authoritative)

| Buffer | Format | Size | Description |
|---|---|---|---|
| `chunk_resident_flags` | `array<u32>` | N slots | `is_resident` bit — set by CPU slot director on load/evict; never written by edit kernels or rebuild passes |

Note: `is_empty` and `has_emissive` are derived summary flags that live in `chunk_flags` (see Derived Summary section below), not here. They are written by the summary rebuild pass from occupancy, not by producers.

### Derived Summary (written by summary rebuild pass)

| Buffer | Format | Size | Description |
|---|---|---|---|
| `chunk_flags` | `array<u32>` | N slots | `is_empty`, `has_emissive`, and other occupancy-derived flags; rebuilt whenever occupancy changes |

### Change Detection (per-frame, reset between frames)

| Buffer | Format | Size | Description |
|---|---|---|---|
| `dirty_chunks` | `array<atomic<u32>>` | ceil(N/32) u32 | One bit per slot |
| `dirty_subregions` | `array<atomic<u32>>` | N × 16 u32 | 512 subregion bits per slot |
| `boundary_touch_mask` | `array<atomic<u32>>` | N u32 | 6 face bits per slot |

### Staleness Tracking

| Buffer | Format | Size | Description |
|---|---|---|---|
| `stale_mesh` | `array<u32>` | ceil(N/32) u32 | One bit per slot; set by propagation pass |
| `stale_summary` | `array<u32>` | ceil(N/32) u32 | One bit per slot; set by propagation pass |
| `stale_lighting` | `array<u32>` | ceil(N/32) u32 | One bit per slot; set by propagation pass |
| `stale_meshlet` | `array<u32>` | ceil(N/32) u32 | One bit per slot; set by **mesh rebuild pass** (not propagation pass) after committing a new mesh |

### Work Queues

| Buffer | Format | Size | Description |
|---|---|---|---|
| `mesh_rebuild_queue` | `array<u32>` | N u32 | Compacted slot indices |
| `summary_rebuild_queue` | `array<u32>` | N u32 | Compacted slot indices |
| `lighting_update_queue` | `array<u32>` | N u32 | Compacted slot indices |
| `meshlet_rebuild_queue` | `array<u32>` | N u32 | Compacted slot indices; populated from `stale_meshlet` by compaction pass |
| `queue_counts` | `array<atomic<u32>>` | 4 u32 | One counter per queue |
| `indirect_dispatch_args` | `array<u32>` | 3 u32 × passes | Built from queue_counts for GPU-indirect dispatch |
| `ready_to_swap` | `array<u32>` | ceil(N/32) u32 | One bit per slot; set by mesh/summary rebuild passes; read and cleared by swap pass |
| `ready_to_swap_meshlet` | `array<u32>` | ceil(N/32) u32 | One bit per slot; set by meshlet build pass when output is valid; read and cleared by swap pass |

### Derived Version Tags

| Buffer | Format | Size | Description |
|---|---|---|---|
| `mesh_version` | `array<u32>` | N slots | chunk_version at mesh build time |
| `summary_version` | `array<u32>` | N slots | chunk_version at summary build time |
| `meshlet_version` | `array<u32>` | N slots | chunk_version at meshlet build time |
| `gi_cache_version` | `array<u32>` | N slots | chunk_version at GI cache build time |

---

## What Producers Must Do

Every producer that writes voxel data must:

1. Write `chunk_occupancy_atlas` and/or `chunk_palette_buf` / `chunk_index_buf`
2. `atomicAdd(&chunk_version[slot], 1)`
3. `atomicOr(&dirty_chunks[slot>>5], 1u << (slot&31))`
4. `atomicOr(&dirty_subregions[slot*16 + word], bits)` for affected subregions
5. `atomicOr(&boundary_touch_mask[slot], face_bits)` if write was within 1 voxel of chunk boundary

Nothing else. The propagation pass handles everything downstream.

Producers must not:
- Write to stale bitsets directly
- Write to work queues
- Write to derived products
- Attempt to synchronously refresh any derived structure

All of that happens downstream of the edit kernel.

---

## See Also

- [chunk-field-registry](chunk-field-registry.md) — authoritative field definitions; version field spec
- [chunk-contract](chunk-contract.md) — edit semantics (CPU-side), boundary propagation rules
- [extension-seams](extension-seams.md) — why derived data is never edited directly (Invariant, the database analogy)
- [gpu-chunk-pool](gpu-chunk-pool.md) — slot allocation; how `slot` indices map to world chunks
- [pipeline-stages](pipeline-stages.md) — where edit kernels and rebuild passes sit in the frame
