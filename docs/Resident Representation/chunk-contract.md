# Canonical Runtime Chunk Contract

**Type:** spec
**Status:** current
**Date:** 2026-03-21

This document defines the chunk as the authoritative unit of voxel-space truth at runtime.
All other structures are either producers that write into chunks, or consumers that read from them.

---

## The Three Layers

```
┌─────────────────────────────────────────────┐
│  PRODUCERS (transient, write-once per job)  │
│  brick voxelizer · procedural gen · edits   │
└────────────────────┬────────────────────────┘
                     │ write
                     ▼
┌─────────────────────────────────────────────┐
│  CANONICAL RUNTIME (persistent, versioned)  │
│              chunk database                 │
└────────────────────┬────────────────────────┘
                     │ read (derived)
                     ▼
┌─────────────────────────────────────────────┐
│  CONSUMERS (derived, rebuildable)           │
│  greedy mesh · A&W traversal · GI · Hi-Z    │
└─────────────────────────────────────────────┘
```

Nothing bypasses the middle layer. CompactVoxel[] is a courier into it, not part of it.

---

## Chunk Identity

```
ChunkCoord { x: i32, y: i32, z: i32 }
```

- Chunk-space indices. Supports negative coordinates via Euclidean division.
- World origin of a chunk: `coord * CS * voxel_size` where CS = 62 (usable voxels per axis)
- Chunk size in voxels: 64³ with 1-voxel padding border; usable interior is 62³

---

## Field Contract

For each field, the contract specifies:
- **Class**: authoritative | derived | metadata
- **Residency (current)**: CPU-only
- **Residency (target)**: CPU-mirrored | GPU-resident | GPU-only
- **Required for**: which consumers need it
- **Rebuildable**: can be recomputed from authoritative data alone

---

### Authoritative Fields

These are the source of truth. No consumer may override them. Producers write here and nowhere else.

#### `opaque_mask: [u64; 4096]`

| Property | Value |
|---|---|
| Class | **Authoritative** |
| Residency (current) | CPU-only (Rust ChunkManager, Web Worker) |
| Residency (target) | **GPU-resident** + CPU-mirrored |
| Required for | Greedy meshing · A&W traversal · GI raymarching · Hi-Z culling |
| Rebuildable | No — this IS the data |

Layout: `opaque_mask[x * 64 + z]` is a u64 column of Y-axis bits. Bit `y` in that column is 1 if the voxel at `(x, y, z)` is occupied.

The 1-voxel padding border (columns 0 and 63 on each axis) stores neighbor boundary data. The usable interior is indices 1–62.

GPU target: a `storage` buffer (`array<u32>`) or `r32uint` 3D texture per chunk. The 3D texture is preferred for traversal (hardware cache locality). The storage buffer is preferred for atomic writes during GPU-side editing.

#### `materials: PaletteMaterials`

| Property | Value |
|---|---|
| Class | **Authoritative** |
| Residency (current) | CPU-only |
| Residency (target) | GPU-resident (palette buffer + index buffer per chunk) |
| Required for | Greedy meshing · GI emissive queries · future material queries |
| Rebuildable | No — material assignment is ground truth |

Internally: a palette of unique `MaterialId` values + bitpacked per-voxel palette indices. Bit width auto-scales with palette size (1–16 bits per voxel).

GPU target: two buffers per chunk:
- `palette_buf: array<u32>` — material IDs (max 64K entries)
- `index_buf: array<u32>` — bitpacked voxel→palette indices

#### `coord: ChunkCoord`

| Property | Value |
|---|---|
| Class | **Authoritative** |
| Residency (current) | CPU |
| Residency (target) | CPU + GPU uniform/push constant per draw call |
| Required for | All consumers (world-space coordinate reconstruction) |
| Rebuildable | No — identity |

#### `data_version: u64`

| Property | Value |
|---|---|
| Class | **Authoritative** (metadata) |
| Residency (current) | CPU |
| Residency (target) | CPU only |
| Required for | Version conflict detection during async meshing |
| Rebuildable | No — monotonic counter, reset on eviction |

Incremented on every voxel write. Captured by async mesh jobs to detect stale results.

---

### Derived Fields

Derived fields are computed from authoritative data and can be discarded and rebuilt at any time.
They may be cached for performance but must never be treated as ground truth.

#### `mesh: ChunkMesh`

| Property | Value |
|---|---|
| Class | **Derived** |
| Residency (current) | CPU → GPU (Three.js BufferGeometry) |
| Residency (target) | GPU-only (vertex/index buffer pool) |
| Required for | Rasterization |
| Rebuildable | Yes — from `opaque_mask` + `materials` via greedy mesher |

Contains: positions, normals, indices, UVs, material IDs per vertex.

#### `face_masks: FaceMasks` (transient)

| Property | Value |
|---|---|
| Class | **Derived** (transient) |
| Residency (current) | CPU stack during meshing |
| Residency (target) | GPU compute intermediate (never persisted) |
| Required for | Greedy meshing only |
| Rebuildable | Yes — from `opaque_mask` in one bitwise pass |

Intermediate result of face culling. Not stored after meshing completes.

#### `state: ChunkState` (metadata)

| Property | Value |
|---|---|
| Class | **Derived** (metadata) |
| Residency (current) | CPU |
| Residency (target) | CPU only |
| Required for | Async mesh job lifecycle (Dirty / Meshing / ReadyToSwap / Clean) |
| Rebuildable | Partially — state resets to Dirty on load |

---

### Planned Derived Fields (not yet implemented)

These fields do not exist today. They are called out here because future consumers (A&W traversal, GI, culling) will require them, and they should attach to chunks — not to bricks, meshes, or any other structure.

#### `occupancy_summary` — coarse-scale traversal acceleration

Purpose: allow A&W and GI probe traversal to skip empty sub-regions within a chunk without testing every voxel.

Candidates:
- **8³ bricklet grid** — divide each 62³ chunk into up to 8³ = 512 bricklets of ~8³ voxels each; store one bit per bricklet (is any voxel occupied?)
- **4-level mip pyramid** — 62³ → 32³ → 16³ → 8³ → 4³ occupancy masks, 1 bit per cell at each level

Either form is derived from `opaque_mask` and rebuilds whenever the chunk is dirtied.

GPU target: packed `array<u32>` alongside the per-voxel occupancy. A DDA traversal first tests the coarse level, skips to next occupied bricklet, then recurses into per-voxel.

#### `chunk_flags: u32` — per-chunk summary bits

A single word of derived boolean summaries for fast GPU-side decisions:

| Bit | Meaning | Source |
|---|---|---|
| 0 | `is_empty` — no occupied voxels | `opaque_mask` all zero |
| 1 | `is_fully_opaque` — all voxels occupied | `opaque_mask` all one |
| 2 | `has_emissive` — any voxel with emissive material | `palette` contains emissive entry |
| 3 | `is_resident` — chunk is in GPU memory | residency manager |
| 4–7 | reserved | — |

GPU target: a flat `array<u32>` indexed by chunk slot, updated whenever `opaque_mask` or `materials` changes. Readable by culling and GI compute shaders.

#### `aabb: (vec3f, vec3f)` — tight world-space bounds

Derived from `opaque_mask` — the smallest AABB containing all occupied voxels. Used for:
- GPU frustum culling (tighter than chunk boundary)
- Hi-Z occlusion culling input

---

## Edit Semantics

### Single-voxel edit
```
set_voxel(world_pos, material):
  1. Resolve ChunkCoord from world_pos
  2. Get-or-create chunk
  3. Write bit in opaque_mask, update palette
  4. Increment data_version
  5. Mark chunk Dirty
  6. If voxel is on chunk boundary: mark adjacent chunk(s) Dirty
```

Boundary marking is required because face visibility depends on the neighbor.
Only the 6 direct neighbors are marked — no cascading.

### Bulk ingest
```
ingest(compact_voxels: &[CompactVoxel]):
  For each voxel: resolve chunk, write, mark boundary neighbors
  Deduplication: DirtyTracker is a HashSet — N edits to same chunk = 1 rebuild
```

### What must be invalidated on any edit

| Field | Invalidation action |
|---|---|
| `opaque_mask` | Updated in place (write) |
| `materials` | Updated in place (write) |
| `data_version` | Incremented |
| `mesh` | Discarded; chunk enters Dirty state |
| `face_masks` | Transient; recomputed at next mesh |
| `occupancy_summary` | Rebuilt after next mesh or separately |
| `chunk_flags` | Recomputed (cheap, one pass over opaque_mask) |
| `aabb` | Recomputed (tight bounds may change) |

---

## Residency Contract

Chunks are not all in memory simultaneously. The residency system manages which chunks are loaded.

**Current state:**
- Resident = in the Rust `ChunkManager` HashMap (CPU)
- Non-resident = evicted; must be re-populated from a producer on next access

**Target state (GPU-resident):**
- A fixed-size GPU **chunk pool** (e.g., 1024 slots × chunk data per slot)
- `chunk_flags` and `occupancy_summary` in flat GPU arrays indexed by slot
- `opaque_mask` and `materials` in per-slot GPU buffers or 3D texture atlas
- CPU-side slot table: `HashMap<ChunkCoord, SlotIndex>`
- Residency manager handles slot allocation, eviction (LRU), and upload

**GPU-resident does not mean CPU-free.** For editing, the CPU-mirrored copy remains authoritative. GPU buffers are synchronized from CPU after a dirty rebuild. Long-term, GPU-side editing could write directly into GPU buffers and sync back to CPU asynchronously — but that is a future concern.

---

## What This Document Locks In

1. **Chunks are the canonical runtime structure.** Bricks are not. Meshes are not. CompactVoxel[] is not.

2. **Producers write into chunks.** CompactVoxel[] is a courier format, not an interchange protocol.

3. **Consumers read from chunks.** Meshing, traversal, GI, culling, streaming — all derive their inputs from `opaque_mask` + `materials` + planned summaries.

4. **Derived data is always rebuildable.** Nothing in the derived layer is authoritative. Discarding and rebuilding is always safe.

5. **Future GPU systems attach to chunks, not to bricks.** The traversal acceleration structures, the probe occupancy inputs, the Hi-Z inputs — all hang off chunk data, not off voxelizer intermediates.

---

## See Also

- [layer-model](layer-model.md) — full producer / canonical / consumer layer definitions
- [gpu-chunk-pool](gpu-chunk-pool.md) — GPU residency pool design (slot allocation, atlas layout, upload protocol)
- [traversal-acceleration](traversal-acceleration.md) — occupancy summary and bricklet design for A&W / GI
- [edit-protocol](edit-protocol.md) — full edit propagation semantics including GPU-side invalidation
