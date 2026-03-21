# GPU Chunk Pool

**Type:** spec
**Status:** current
**Date:** 2026-03-21

Slot allocation, atlas layout, and CPU↔GPU sync for the GPU-resident chunk runtime.

---

## The Central Distinction

> **GPU-driven scheduling simplifies the control plane more than the storage plane.**

Before designing the chunk pool, separate the problem in two:

**Control plane** — Who notices work, builds lists, dispatches kernels, tracks readiness?
GPU-driven scheduling helps a lot here. Dirty detection, queue compaction, indirect dispatch generation, stale propagation — these are small local decisions made on data already on the GPU. Moving them GPU-side eliminates readbacks, sync points, and CPU micromanagement.

**Data plane** — Where do chunks live, how are pages assigned, how are handles kept stable, how is fragmentation managed?
GPU-driven scheduling helps only indirectly here. The storage problems are allocator problems, not scheduler problems. Expecting the scheduler to solve them for free is how the temple catches fire.

The chunk pool design must address both, but must not conflate them.

---

## What GPU-Driven Scheduling Actually Simplifies

For the three stated problems:

| Problem | GPU-driven helps? | Why |
|---|---|---|
| **GPU slot allocation** | Partially — transient allocation yes, persistent lifetime no | Atomics + prefix sums handle queue slot assignment efficiently. Long-lived chunk slots with fragmentation/lifetime concerns are still allocator problems. |
| **Atlas layout** | Operationally, not conceptually | GPU can drive page update worklists and copy passes. It does not simplify how the atlas is partitioned, how fragmentation is handled, or how stable handles are maintained. |
| **CPU↔GPU sync protocol** | Yes, significantly | If the GPU detects dirtiness, compacts work, and generates indirect dispatch args, the CPU no longer reads back dirty lists or micromanages state transitions. Fewer readbacks, fewer sync points, simpler outer loop. |

The sync story is where GPU-driven scheduling earns its keep most clearly. The storage story still needs a carefully designed model.

---

## Buffer Classification: Control Plane vs. Data Plane

Every buffer in the GPU-resident runtime belongs to one plane. Mixing them is the source of most pool design confusion.

### Control Plane Buffers
*Who drives scheduling: small, frequently written, GPU-side decisions*

| Buffer | Owner | Written by | Read by |
|---|---|---|---|
| `dirty_chunks` | Edit protocol | Edit kernels (atomic) | Propagation pass |
| `dirty_subregions` | Edit protocol | Edit kernels (atomic) | Propagation pass |
| `boundary_touch_mask` | Edit protocol | Edit kernels (atomic) | Propagation pass |
| `stale_mesh` | Edit protocol | Propagation pass | Compaction pass |
| `stale_summary` | Edit protocol | Propagation pass | Compaction pass |
| `stale_lighting` | Edit protocol | Propagation pass | Compaction pass |
| `mesh_rebuild_queue` | Edit protocol | Compaction pass | Mesh rebuild pass |
| `summary_rebuild_queue` | Edit protocol | Compaction pass | Summary rebuild pass |
| `lighting_update_queue` | Edit protocol | Compaction pass | Lighting pass |
| `queue_counts` | Edit protocol | Compaction pass (atomic) | CPU (Stage 2) or indirect args |
| `indirect_dispatch_args` | Edit protocol | Compaction pass | `dispatchWorkgroupsIndirect` |
| `chunk_version` | Chunk pool | Edit kernels (atomic) | All derived consumers |
| `mesh_version` | Chunk pool | Mesh rebuild pass | Swap pass |
| `summary_version` | Chunk pool | Summary rebuild pass | Swap pass |
| `ready_to_swap` | Chunk pool | Rebuild passes | Swap pass |

These buffers are frequently written, typically small, and their logic benefits from being GPU-side. The edit-protocol's migration ladder applies here.

### Data Plane Buffers
*Where stuff lives: large, long-lived, allocator-managed*

| Buffer | Owner | Written by | Read by |
|---|---|---|---|
| `chunk_occupancy_atlas` | Chunk pool | CPU upload / edit kernels | Traversal, meshing, summaries |
| `chunk_palette_buf` | Chunk pool | CPU upload / edit kernels | Traversal, meshing |
| `chunk_index_buf` | Chunk pool | CPU upload / edit kernels | Traversal, meshing |
| `chunk_resident_flags` | Chunk pool | CPU slot director (on load/evict) | Residency checks |
| `chunk_flags` | Chunk pool | Summary rebuild pass | Traversal, culling |
| `occupancy_summary` | Chunk pool | Summary rebuild pass | Traversal |
| `chunk_aabb` | Chunk pool | Summary rebuild pass | Culling |
| `vertex_pool` | Mesh pool | Mesh rebuild pass | Raster passes |
| `index_pool` | Mesh pool | Mesh rebuild pass | Raster passes |
| `draw_metadata` | Mesh pool | Mesh rebuild pass | Cull pass, indirect draw |
| `chunk_coord` | Chunk pool | CPU (on load) | All consumers |
| `chunk_slot_table_gpu` | Chunk pool | CPU (on load/evict) | Traversal coord→slot lookup |

These buffers are long-lived, large, and their design is an allocator problem. GPU-driven scheduling operates on top of them but does not define their structure.

---

## Slot Allocation Design

### The Pool

A fixed-size array of N slots. Each slot holds all per-chunk GPU data for one resident chunk.

```
N = max_resident_chunks   (e.g., 1024 for a typical scene)

Per slot:
  chunk_occupancy_atlas[slot]   2048 u32  = 8KB           (authoritative)
  chunk_palette_buf[slot]       variable  (max 64K × 4B = 256KB, typical << 1KB)  (authoritative)
  chunk_index_buf[slot]         variable  (depends on palette bit width)           (authoritative)
  palette_meta[slot]            1 u32     palette_size (u16) + bits_per_entry (u8) + reserved (u8) — CPU-written (authoritative)
  chunk_resident_flags[slot]    1 u32     is_resident bit — CPU-written on load/evict (authoritative)
  chunk_flags[slot]             1 u32     is_empty, has_emissive — summary rebuild pass (derived)
  occupancy_summary[slot]       16 u32                    (derived)
  chunk_aabb[slot]              2 × vec4f                 (derived)
  chunk_coord[slot]             vec4i                     (authoritative)
  chunk_version[slot]           u32                       (authoritative)
  mesh_version[slot]            u32                       (control plane)
  summary_version[slot]         u32                       (control plane)
```

### Slot Directory (CPU-side)

The slot directory is CPU-managed. The GPU does not allocate or free slots. The GPU reads slot indices from lookup structures but does not modify the directory.

```
slot_table: HashMap<ChunkCoord, SlotIndex>   // CPU-only
free_slots: Vec<SlotIndex>                    // CPU-managed freelist
```

**Why CPU manages the directory:**
Slot allocation involves long-lived ownership semantics, eviction policy (LRU), memory pressure decisions, and fallback behavior. These are policy-heavy decisions that belong to the orchestration layer. The CPU is the right owner.

**What GPU does with slots:**
- Looks up slots by coord via `chunk_slot_table_gpu` (a GPU-resident flat array updated by CPU on load/evict)
- Reads slot-indexed data for rendering and traversal
- Writes back to slot-indexed data during rebuilds

### Slot Lifecycle

```
Load:
  CPU: allocate slot from free_slots
  CPU: record in slot_table[coord] = slot
  CPU: writeBuffer → chunk_coord[slot], chunk_version[slot] = 0
  CPU: upload occupancy + materials to slot
  CPU: update chunk_slot_table_gpu[slot] with coord→slot mapping
  CPU: set chunk_resident_flags[slot].is_resident = 1
  CPU: set stale_summary bit for this slot (triggers compaction → summary_rebuild_queue)
  Note: CPU must NOT write directly to summary_rebuild_queue. Queues are populated
        exclusively by the GPU compaction pass from stale bitsets (see edit-protocol).

Evict:
  CPU: check slot is not in active rebuild queue (ready_to_swap bit is clear)
  CPU: clear chunk_slot_table_gpu entry
  CPU: clear chunk_resident_flags[slot].is_resident = 0
  CPU: return slot to free_slots
  CPU: remove from slot_table

Edit (GPU-side):
  Edit kernel: write occupancy/materials in slot
  Edit kernel: atomicAdd chunk_version[slot]
  Edit kernel: set dirty bits (control plane)
```

### Occupancy Atlas Layout Options

Two viable layouts for `chunk_occupancy_atlas`:

**Option A — Flat storage buffer per slot (recommended for now)**
```
chunk_occupancy: array<array<u32, 2048>, N_SLOTS>
```
- Simple addressing: `occupancy[slot * 2048 + word_index]`
- Uniform slot size (64³ / 32 = 2048 u32 per chunk)
- No hardware texture cache, but cache-friendly for A&W column access along X/Z axis
- Easy to debug

**Option B — 3D texture atlas (future)**
```
chunk_occupancy_atlas: texture3d<r32uint>
  dimensions: 64 × 64 × (64 × N_SLOTS)  // stacked along Z
```
- Hardware texture cache benefits for spatially coherent access patterns
- Required if traversal shader uses `textureLoad` rather than buffer indexing
- More complex addressing; requires slot→Z-offset lookup
- Higher initial implementation cost

Start with Option A. Migrate to Option B if traversal profiling shows cache miss pressure.

---

## Atlas Layout for Derived Products

Derived summary buffers use the same slot indexing as occupancy. No separate atlas management needed — they are flat arrays indexed by slot.

```
chunk_flags[N_SLOTS]              array<u32>
occupancy_summary[N_SLOTS × 16]   array<u32>   (16 u32 = 512 bits per chunk)
chunk_aabb[N_SLOTS × 2]           array<vec4f>
```

These are cheap enough that all N slots are always allocated, regardless of residency. The `is_resident` flag in `chunk_resident_flags` gates whether a slot's data is valid.

---

## Mesh Pool Design

Mesh geometry (vertices + indices) is stored in a separate pool with variable-size regions per chunk.

```
vertex_pool: array<f32>    // positions + normals packed, all chunks
index_pool:  array<u32>    // indices, all chunks
draw_metadata: array<DrawMetadata>   // per slot: vertex offset, index offset, count, coord
```

### Variable-Size Allocation Problem

Unlike the occupancy pool (fixed 2048 u32 per slot), mesh sizes vary widely — an empty chunk has zero vertices, a dense chunk may have tens of thousands.

Two approaches:

**Option A — Fixed worst-case reservation per slot**
- Reserve `MAX_VERTS_PER_CHUNK` and `MAX_INDICES_PER_CHUNK` per slot
- Simple, no fragmentation, but wasteful (50–90% of reserved space typically empty)
- Acceptable for a bounded pool (1024 slots × 65K verts × 12B = ~800MB worst case; typical scene << 10% of that)

**Option B — Variable allocation with a freelist**
- Mesh regions allocated from a large flat buffer using a GPU-side or CPU-side allocator
- More efficient but requires compaction strategy for fragmentation
- Deferred — implement Option A first, move to Option B when memory pressure becomes measurable

### Fragmentation Note

Mesh fragmentation accumulates when chunks are frequently rebuilt (edits). Compaction is a future concern. For a testbed where scenes are loaded once and lightly edited, Option A with periodic full-pool rebuild on scene change is sufficient.

---

## CPU↔GPU Sync Protocol

This is where GPU-driven scheduling provides the clearest architectural win.

### What must cross the CPU↔GPU boundary

| Data | Direction | Frequency | Size |
|---|---|---|---|
| Occupancy + materials (new chunk) | CPU → GPU | On chunk load | ~8KB per chunk |
| Occupancy + materials (edit) | CPU → GPU | On CPU-side edit | ~8KB per chunk |
| `chunk_slot_table_gpu` entry | CPU → GPU | On load/evict | 16 bytes |
| `frame_budget` uniform | CPU → GPU | Every frame (Stage 3) | 4 bytes |
| `queue_counts` readback | GPU → CPU | Every frame (Stage 2 only) | 16 bytes |

At Stage 3, the only CPU → GPU write per frame is the budget uniform. The GPU → CPU readback is eliminated. The sync story becomes: CPU submits command buffer, GPU executes, no stalls.

### Eliminating the Stall

The current Rust ChunkManager stalls on GPU→CPU readback after voxelization (`device.poll(Maintain::Wait)`). In the GPU-resident target:

1. Voxelizer writes directly into `chunk_occupancy_atlas[slot]` on GPU
2. Edit kernels write dirty bits immediately after
3. No readback, no stall, no `poll(Wait)`
4. CPU is notified of completion via fence / async callback (WebGPU `onSubmittedWorkDone`)

The fence callback is the only sync point — and it signals completion, not data transfer. The CPU does not receive voxel data; it receives a signal that the slot is ready for rendering.

### Stable Handles Across Async Work

A common failure: async rebuild completes after eviction. The slot has been reassigned to a different chunk. The rebuild result must not be written to the now-reassigned slot.

Solution: version tagging (from [edit-protocol](edit-protocol.md)).

```
At rebuild dispatch:
  captured_version = chunk_version[slot]

At rebuild completion:
  if chunk_version[slot] == captured_version && slot_table[coord] == slot:
      write result
      mark ready_to_swap
  else:
      discard
```

The double check (version and slot-still-owned) prevents both stale-version and eviction-reuse corruption.

---

## Memory Budget

Default configuration for a desktop WebGPU session:

| Resource | Per slot | 1024 slots |
|---|---|---|
| Occupancy atlas (Option A) | 8KB | 8MB |
| Palette + index buf | ~2KB typical | ~2MB |
| Derived summaries | ~1KB | ~1MB |
| Mesh (Option A, fixed) | ~256KB typical | ~256MB |
| Control plane buffers | — | ~1MB total |
| **Total** | | **~268MB** |

Well within a modern GPU's budget. Reduce `N_SLOTS` to 512 for lower-end devices.

---

## What Still Needs Design

| Component | Status | Notes |
|---|---|---|
| Slot directory (CPU) | Specified here | Implements LRU eviction from existing ChunkManager logic |
| Occupancy atlas (Option A, flat buffer) | Specified here | First implementation target |
| `chunk_slot_table_gpu` update protocol | Specified here | CPU writes on load/evict |
| Mesh pool (Option A, fixed reservation) | Specified here | First implementation target |
| Variable mesh allocation (Option B) | Deferred | After Option A and profiling |
| 3D texture atlas (Option B) | Deferred | After traversal profiling |
| GPU-side slot compaction | Deferred | Only needed if fragmentation becomes measurable |
| Palette variable-length allocation | Needs design | Palettes vary in size; current spec assumes bounded max |

---

## See Also

- [edit-protocol](edit-protocol.md) — control plane buffers, dirty/stale tracking, work queues
- [chunk-field-registry](chunk-field-registry.md) — data plane fields and their classification
- [traversal-acceleration](traversal-acceleration.md) — how traversal reads from the occupancy atlas (Stage R-6)
- [pipeline-stages](pipeline-stages.md) — where pool data is consumed in the frame
- [extension-seams](extension-seams.md) — why slot allocation is a storage problem, not a scheduling problem
