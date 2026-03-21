# Gestalt Architecture Map — Data Structures, Algorithms, and Relationships

**Type:** reference
**Status:** current
**Date:** 2026-03-09

---

## Purpose

This document maps every data structure and algorithm in the Gestalt codebase,
shows how they relate to each other today, and identifies shared functionality
and critical core logic that underpins multiple features.

---

## System Overview

```
                    ┌─────────────────────────────────────────────────────────┐
                    │                    USER INPUT                           │
                    │        (OBJ file, procedural pattern, voxel edit)       │
                    └──────────────────────┬──────────────────────────────────┘
                                           │
                    ┌──────────────────────▼──────────────────────────────────┐
                    │              GEOMETRY INGESTION                          │
                    │                                                         │
                    │  OBJ Parser ──▶ MeshInput{triangles, material_ids}      │
                    │  Procedural  ──▶ Dense voxel array                      │
                    │  User Edit   ──▶ set_voxel(coord, material)             │
                    └──────────────────────┬──────────────────────────────────┘
                                           │
                    ┌──────────────────────▼──────────────────────────────────┐
                    │              VOXELIZATION (GPU)                          │
                    │                                                         │
                    │  GpuVoxelizer ──▶ SparseVoxelizationOutput              │
                    │  {occupancy bits, owner_id, color_rgba, brick_origins}  │
                    │                                                         │
                    │  Future (ADR-0009): ──▶ CompactVoxel[]                  │
                    │  {global_vx, global_vy, global_vz, MaterialId}          │
                    └──────────────────────┬──────────────────────────────────┘
                                           │
                    ┌──────────────────────▼──────────────────────────────────┐
                    │              CHUNK MANAGER (canonical voxel store)       │
                    │                                                         │
                    │  ChunkManager ──▶ HashMap<ChunkCoord, Chunk>            │
                    │  Chunk ──▶ BinaryChunk{opaque_mask, PaletteMaterials}   │
                    │                                                         │
                    │  Services: dirty tracking, LRU eviction, memory budget, │
                    │           frame-budgeted rebuild, version consistency    │
                    └──────────────────────┬──────────────────────────────────┘
                                           │
              ┌────────────────────────────┼────────────────────────────┐
              │                            │                            │
              ▼                            ▼                            ▼
┌──────────────────────┐   ┌──────────────────────┐   ┌──────────────────────┐
│  GREEDY MESHING       │   │  OCCUPANCY UPLOAD     │   │  CHUNK BOUNDS        │
│                       │   │  (future)             │   │  (future)            │
│  BinaryChunk          │   │                       │   │                      │
│  ──▶ cull_faces()     │   │  opaque_mask[]        │   │  ChunkCoord          │
│  ──▶ greedy_merge()   │   │  ──▶ 3D texture       │   │  ──▶ AABB buffer     │
│  ──▶ expand_quads()   │   │  (for radiance        │   │  (for Hi-Z culling)  │
│  ──▶ MeshOutput       │   │   cascade raymarch)   │   │                      │
└──────────┬───────────┘   └──────────┬───────────┘   └──────────┬───────────┘
           │                          │                           │
           ▼                          ▼                           ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                    GPU-DRIVEN RENDERING PIPELINE (ADR-0011)                   │
│                                                                              │
│  ┌─────────┐   ┌─────────┐   ┌─────────┐   ┌──────────┐   ┌─────────────┐  │
│  │ Depth   │──▶│ Hi-Z    │──▶│ Cull    │──▶│ Cascade  │──▶│ Color Pass  │  │
│  │ Prepass │   │ Pyramid │   │ Compute │   │ Build    │   │ + GI        │  │
│  └─────────┘   └─────────┘   └─────────┘   └──────────┘   └─────────────┘  │
│                                                                              │
│  Inputs: MeshOutput buffers, AABB bounds, 3D occupancy, MaterialDef atlas    │
│  Output: Final lit frame                                                     │
└──────────────────────────────────────────────────────────────────────────────┘
           │
           ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                    THREE.JS OVERLAY (debug helpers, UI, WebGL2 fallback)      │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## Complete Data Structure Inventory

### Tier 1: Core Data (used by 3+ systems)

These structures are the foundation. Changes to them ripple across the entire
project.

| Structure | Location | Used by | Fields |
|-----------|----------|---------|--------|
| **BinaryChunk** | `greedy_mesher/src/core.rs:48` | Meshing, chunk manager, occupancy upload, radiance cascades | `opaque_mask: [u64; 4096]`, `materials: PaletteMaterials` |
| **MaterialId** | `greedy_mesher/src/core.rs:5` | Meshing, materials, voxelizer integration, rendering | `u16` (0=empty, 1=default, 2-65535=user) |
| **ChunkCoord** | `greedy_mesher/src/chunk/coord.rs:10` | Chunk manager, dirty tracking, LRU, rebuild queue, culling, rendering | `x: i32, y: i32, z: i32` |
| **MeshOutput** | `greedy_mesher/src/core.rs:261` | Meshing, WASM bindings, rendering, buffer management, cluster metadata | `positions, normals, indices, uvs, material_ids` |
| **ChunkManager** | `greedy_mesher/src/chunk/manager.rs:25` | All chunk operations, voxelizer ingestion, dirty tracking, meshing trigger | `chunks, dirty_tracker, rebuild_queue, config, lru_tracker, budget` |

### Tier 2: Algorithm-Specific Data

| Structure | Location | Used by | Fields |
|-----------|----------|---------|--------|
| **FaceMasks** | `greedy_mesher/src/core.rs:211` | Face culling, greedy merge | `masks: [u64; 24576]` (6 directions × CS_P²) |
| **PaletteMaterials** | `greedy_mesher/src/chunk/palette_materials.rs:44` | BinaryChunk material storage | `palette: Vec<MaterialId>, indices: Vec<u64>, bits_per_voxel: u8` |
| **VoxelGridSpec** | `voxelizer/src/core.rs:4` | GPU voxelization, coordinate conversion | `origin_world, voxel_size, dims, world_to_grid` |
| **SparseVoxelizationOutput** | `voxelizer/src/core.rs:143` | GPU voxelizer output, brick iteration | `brick_dim, brick_origins, occupancy, owner_id, color_rgba, stats` |
| **GpuVoxelizer** | `voxelizer/src/gpu/mod.rs:87` | GPU voxelization pipeline | `device, queue, pipeline, bind_group_layout, ...` |
| **TileTriangleCsr** | `voxelizer/src/csr.rs:28` | Spatial indexing for voxelization | `tile_offsets, tri_indices, tri_counts` |

### Tier 3: State Management

| Structure | Location | Used by | Fields |
|-----------|----------|---------|--------|
| **Chunk** | `greedy_mesher/src/chunk/chunk.rs:84` | Chunk manager, meshing, rendering | `coord, state, data_version, voxels, mesh, pending_mesh` |
| **ChunkState** | `greedy_mesher/src/chunk/state.rs:8` | Chunk lifecycle | `Clean, Dirty, Meshing{ver}, ReadyToSwap{ver}` |
| **ChunkMesh** | `greedy_mesher/src/chunk/chunk.rs:11` | Chunk render data | `positions, normals, indices, uvs, material_ids, data_version` |
| **DirtyTracker** | `greedy_mesher/src/chunk/dirty.rs:12` | Chunk manager, rebuild scheduling | `dirty_chunks: HashSet<ChunkCoord>` |
| **RebuildQueue** | `greedy_mesher/src/chunk/queue.rs:48` | Frame-budgeted meshing | `queue: BinaryHeap<RebuildRequest>, in_queue: HashSet<ChunkCoord>` |
| **LruTracker** | `greedy_mesher/src/chunk/lru.rs:14` | Memory eviction | `access_times: HashMap<ChunkCoord, u64>, current_time: u64` |
| **MemoryBudget** | `greedy_mesher/src/chunk/budget.rs:14` | Memory management | `max_bytes, high_watermark, low_watermark, min_chunks` |

### Tier 4: Transport / Binding Layer

| Structure | Location | Used by | Fields |
|-----------|----------|---------|--------|
| **MeshResult** (WASM) | `wasm_greedy_mesher/src/lib.rs:21` | JS↔Rust mesh data transfer | mirrors MeshOutput |
| **WasmChunkManager** | `wasm_greedy_mesher/src/lib.rs:412` | JS chunk manager interface | wraps ChunkManager |
| **WasmVoxelizer** | `wasm_voxelizer/src/lib.rs:139` | JS voxelizer interface | wraps GpuVoxelizer |
| **ChunkMeshTransfer** (TS) | `workers/chunkManagerTypes.ts:91` | Worker→main thread mesh data | `coord, positions, normals, indices, uvs, materialIds` |
| **ModuleOutput** (TS) | `modules/types.ts:36` | Module→viewer data flow | `kind: mesh\|voxels\|lines\|points\|texture2d` |
| **VoxelizerAdapter** (TS) | `packages/voxelizer-js/src/index.ts:161` | High-level voxelizer API | wraps WasmVoxelizer |

### Tier 5: Rendering / Viewer

| Structure | Location | Used by | Fields |
|-----------|----------|---------|--------|
| **ViewerBackend** (TS) | `viewer/threeBackend.ts:30` | All rendering | `renderer, scene, camera, controls, isWebGPU` |
| **Viewer** (TS) | `viewer/Viewer.ts:22` | Scene management, output display | `outputGroup, grid, axes, bounds, stats` |
| **OutputStats** (TS) | `viewer/outputs.ts:26` | Performance display | `triangles, instances` |
| **FreeCamControls** (TS) | `viewer/freeCamControls.ts:13` | Camera navigation | `target, keyState, velocity` |

---

## Algorithm Inventory

### Implemented

| Algorithm | Location | Input | Output | Complexity |
|-----------|----------|-------|--------|------------|
| **Bitwise face culling** | `greedy_mesher/src/cull.rs` | BinaryChunk.opaque_mask | FaceMasks | O(CS_P²) per axis |
| **Binary greedy merge** | `greedy_mesher/src/merge/` | FaceMasks | Packed quads (u64) | O(CS_P²) per face direction |
| **Quad expansion** | `greedy_mesher/src/expand.rs` | Packed quads | MeshOutput (verts, tris) | O(quad_count) |
| **Palette compression** | `greedy_mesher/src/chunk/palette_materials.rs` | MaterialId per voxel | Bitpacked indices + palette | O(n) insert/lookup |
| **Dense→binary conversion** | `greedy_mesher/src/convert.rs` | Dense u16 array | BinaryChunk | O(volume) |
| **Positions→binary conversion** | `greedy_mesher/src/convert.rs` | Float32 positions | BinaryChunk | O(n_positions) |
| **GPU surface voxelization** | `voxelizer/src/gpu/` | Triangle mesh + grid spec | Sparse occupancy bitmask | O(triangles × tiles) |
| **CPU reference voxelization** | `voxelizer/src/reference_cpu.rs` | Triangle mesh + grid spec | Dense occupancy | O(triangles × volume) |
| **CSR spatial index build** | `voxelizer/src/csr.rs` | Triangles + tile grid | TileTriangleCsr | O(triangles × tiles) |
| **Sparse compaction** | `voxelizer/src/gpu/compact_attrs.rs` | Dense occupancy | Sparse brick array | O(volume) GPU |
| **LRU eviction** | `greedy_mesher/src/chunk/lru.rs` | Access times | Eviction candidates | O(n log n) sort |
| **Priority rebuild queue** | `greedy_mesher/src/chunk/queue.rs` | Dirty chunks + priorities | Ordered rebuild sequence | O(log n) per op |
| **OBJ parsing** | `wasm_obj_loader/src/lib.rs` | OBJ text | Triangles + materials | O(file_size) |

### Planned (documented but not implemented)

| Algorithm | Documented in | Input | Output |
|-----------|--------------|-------|--------|
| **GPU compact pass** (ADR-0009) | `voxelizer-integration/` | Occupancy + material_table | CompactVoxel[] |
| **Compact→chunk ingestion** | `voxelizer-integration/design/cpu-ingestion.md` | CompactVoxel[] | ChunkManager writes |
| **Hi-Z pyramid build** | `gpu-driven-rendering/spec/frame-graph.md` | Depth texture | Mip chain (conservative min) |
| **AABB occlusion cull** | `gpu-driven-rendering/spec/frame-graph.md` | Pyramid + bounds | Visibility buffer |
| **Radiance cascade raymarch** | `adr/0010-radiance-cascades.md` | Occupancy 3D tex + depth | Cascade atlas (RGBA16F) |
| **Cascade merge** | `adr/0010-radiance-cascades.md` | N cascade layers | Merged radiance field |
| **Temporal reprojection** | `adr/0010-radiance-cascades.md` | Prev frame + camera motion | Blended cascades |
| **Cluster/backface cull** | `gpu-driven-rendering/spec/visibility-buffer.md` | Cluster AABBs + normals | Filtered indirect args |
| **Point cloud LOD** | `adr/0006-lod-strategy.md` | BinaryChunk.opaque_mask | Point positions |
| **Texture atlas lookup** | `adr/0007-material-strategy.md` | UV + MaterialId | Albedo + PBR properties |

---

## Shared Functionality Matrix

This matrix identifies where different features depend on the same core
capability. Cells marked with **CRITICAL** indicate functionality that
multiple planned features cannot work without.

```
                        ┌────────┬────────┬────────┬────────┬────────┬────────┐
                        │ Greedy │ Voxel- │ Hi-Z   │Radiance│ LOD    │Material│
                        │ Mesh   │ izer   │ Cull   │Cascade │        │ Atlas  │
  ┌─────────────────────┼────────┼────────┼────────┼────────┼────────┼────────┤
  │ BinaryChunk         │ READ   │ WRITE  │   -    │ READ   │ READ   │   -    │
  │ .opaque_mask        │        │(future)│        │(future)│(future)│        │
  ├─────────────────────┼────────┼────────┼────────┼────────┼────────┼────────┤
  │ BinaryChunk         │ READ   │ WRITE  │   -    │   -    │   -    │ READ   │
  │ .materials          │        │(future)│        │        │        │(future)│
  ├─────────────────────┼────────┼────────┼────────┼────────┼────────┼────────┤
  │ ChunkCoord          │  USE   │  USE   │  USE   │  USE   │  USE   │   -    │
  │                     │        │(future)│(future)│(future)│(future)│        │
  ├─────────────────────┼────────┼────────┼────────┼────────┼────────┼────────┤
  │ MeshOutput          │ WRITE  │   -    │ READ   │   -    │   -    │ READ   │
  │ (positions/indices) │        │        │(future)│        │        │(future)│
  ├─────────────────────┼────────┼────────┼────────┼────────┼────────┼────────┤
  │ ChunkManager        │  USE   │  USE   │  USE   │  USE   │  USE   │   -    │
  │ (dirty/version/LRU) │        │(future)│(future)│(future)│(future)│        │
  ├─────────────────────┼────────┼────────┼────────┼────────┼────────┼────────┤
  │ Depth texture       │   -    │   -    │**CRIT**│**CRIT**│   -    │   -    │
  │ (app-owned)         │        │        │        │        │        │        │
  ├─────────────────────┼────────┼────────┼────────┼────────┼────────┼────────┤
  │ GPUDevice (shared)  │   -    │  USE   │**CRIT**│**CRIT**│   -    │   -    │
  │                     │        │        │        │        │        │        │
  ├─────────────────────┼────────┼────────┼────────┼────────┼────────┼────────┤
  │ Indirect draw args  │   -    │   -    │ WRITE  │   -    │   -    │   -    │
  │                     │        │        │(future)│        │        │        │
  ├─────────────────────┼────────┼────────┼────────┼────────┼────────┼────────┤
  │ Occupancy 3D tex    │   -    │   -    │   -    │**CRIT**│   -    │   -    │
  │                     │        │        │        │        │        │        │
  ├─────────────────────┼────────┼────────┼────────┼────────┼────────┼────────┤
  │ Chunk AABB bounds   │   -    │   -    │**CRIT**│   -    │  USE   │   -    │
  │                     │        │        │        │        │(future)│        │
  ├─────────────────────┼────────┼────────┼────────┼────────┼────────┼────────┤
  │ MaterialDef props   │   -    │   -    │   -    │ READ   │   -    │**CRIT**│
  │ (emissive, PBR)     │        │        │        │(future)│        │        │
  └─────────────────────┴────────┴────────┴────────┴────────┴────────┴────────┘
```

---

## Critical Shared Infrastructure

These are the components that multiple future features depend on. They should
be built first, in this order.

### 1. App-Owned Depth Texture

**Needed by:** Hi-Z culling (pyramid source), radiance cascades (probe placement),
cluster culling (two-phase occlusion)

**Current state:** Depth is internal to Three.js `renderer.render()` call at
`threeBackend.ts:114`. Not accessible.

**What to build:** Custom depth-only render pass writing to a `GPUTexture`
with `RENDER_ATTACHMENT | TEXTURE_BINDING` usage.

**Documented in:** `gpu-driven-rendering/design/hybrid-transition.md` Phase 1,
`culling/hiz-occlusion-culling-report.md` §4

### 2. Shared GPUDevice Handle

**Needed by:** All custom compute passes (Hi-Z, cascades, culling), custom
render passes (depth prepass, color pass)

**Current state:** Modules get a device via `navigator.gpu.requestAdapter()`.
The renderer has its own device. These are separate.

**What to build:** Extract device from `renderer.backend.device`, store on
`ViewerBackend`, share with custom pipeline.

**Documented in:** `gpu-driven-rendering/design/hybrid-transition.md` Phase 0

### 3. Occupancy Data GPU Upload

**Needed by:** Radiance cascade raymarching (primary), potentially Hi-Z
acceleration (future)

**Current state:** `opaque_mask` data lives in Rust/WASM memory. No GPU
representation exists.

**What to build:** Pack chunk `opaque_mask` data into a 3D `r32uint` texture.
Incremental update on chunk dirty. Address as `(world_vx, world_vy, world_vz)`.

**Source data:** `BinaryChunk.opaque_mask: [u64; 4096]` — 32KB per chunk.
Each u64 column → two u32 words in the texture.

### 4. Chunk AABB Buffer

**Needed by:** Hi-Z occlusion culling (primary), LOD distance checks,
frustum culling

**Current state:** No per-chunk bounds metadata exists. Three.js computes
bounding spheres per mesh internally but these are not exposed.

**What to build:** `Float32Array` buffer with `(center_x, center_y, center_z,
extent_x, extent_y, extent_z)` per chunk. Update on chunk load/unload or
mesh rebuild.

**Source data:** `ChunkCoord` → world position + `BinaryChunk` content bounds.

### 5. Global Mesh Buffer Pool

**Needed by:** Indirect draw (required), custom color pass (required),
cluster culling (future)

**Current state:** Each chunk is a separate `THREE.Mesh` with its own
`BufferGeometry`. No global buffer.

**What to build:** Single vertex buffer + index buffer hosting all chunk
meshes. Per-chunk `DrawIndexedIndirectArgs` with offsets. When a chunk mesh
is rebuilt, update its slot.

**Source data:** `MeshOutput` from greedy mesher → slot in global buffer.

---

## Cross-Cutting Data Flows

### Flow 1: Voxel Edit → Visible Frame (current)

```
User edit
  → ChunkManager.set_voxel(coord, material)
    → Chunk.set_voxel_raw() [writes opaque_mask + palette]
    → DirtyTracker.mark_dirty_with_neighbors()
  → ChunkManager.update() [frame-budgeted]
    → RebuildQueue.pop() [highest priority first]
    → mesh_chunk() [cull → merge → expand]
    → Chunk.mark_ready_to_swap(MeshOutput)
    → Chunk.try_swap_mesh()
  → WasmChunkManager → Worker → ChunkMeshTransfer
  → outputs.ts → THREE.BufferGeometry → THREE.Mesh → render
```

### Flow 2: Voxel Edit → Visible Frame (target, with GPU-driven pipeline)

```
User edit
  → ChunkManager.set_voxel(coord, material)
    → DirtyTracker.mark_dirty_with_neighbors()
    → [same rebuild path as Flow 1]
  → Updated MeshOutput → global mesh buffer pool (slot update)
  → Updated opaque_mask → 3D occupancy texture (region update)
  → Updated AABB → chunk bounds buffer
  → GPU pipeline:
    1. Depth prepass (indirect draw from prev frame's visibility)
    2. Hi-Z pyramid build
    3. Cull compute (AABB vs pyramid → update indirect args)
    4. Cascade compute (raymarch occupancy, merge intervals)
    5. Color pass (indirect draw, material atlas + GI)
    6. Three.js overlay
```

### Flow 3: OBJ Load → Voxelized → Meshed → Rendered (target, with ADR-0009)

```
OBJ file
  → parse_obj() → MeshInput{triangles, material_ids}
  → GpuVoxelizer.voxelize() → [GPU compact pass]
    → CompactVoxel[]{vx, vy, vz, MaterialId}  (ADR-0009)
  → compact_to_chunk_writes()
    → group by div_euclid(vx, CS)
    → ChunkManager.set_voxel_raw() per chunk
    → DirtyTracker.mark_dirty()
  → [continues as Flow 2]
```

---

## Coordinate Systems

Four coordinate spaces are used across the project. Conversion errors between
them are a documented risk (ADR-0008 Gap 6).

| Space | Unit | Range | Used by |
|-------|------|-------|---------|
| **World** | float | arbitrary | Camera, Three.js, user-facing API |
| **Voxel (global)** | integer | arbitrary | Voxelizer output (ADR-0009), chunk coord derivation |
| **Chunk** | integer | arbitrary | ChunkCoord, chunk manager keys |
| **Local (in-chunk)** | integer | [0, CS=62) | BinaryChunk indexing, opaque_mask addressing |

**Conversions:**

```
world → voxel:     voxel = floor(world / voxel_size)        [with epsilon tolerance]
voxel → chunk:     chunk = div_euclid(voxel, CS)            [ChunkCoord::from_voxel]
voxel → local:     local = rem_euclid(voxel, CS)            [Chunk::world_to_local]
chunk → world:     world = chunk * CS * voxel_size           [ChunkCoord::origin_world]
local → opaque_mask index:  (local_x + 1) * CS_P + (local_z + 1)  [+1 for padding]
```

**Critical invariants** (from `voxelizer-integration/spec/invariants.md`):
- C1: occupancy conservation (set_voxel_raw sets correct bit)
- C2: local coordinates in [0, 62)
- C3: material validity (no MATERIAL_EMPTY for solid voxels)
- C4: chunk coordinate round-trip consistency

---

## Constants

These constants are shared across multiple systems. Changing any of them
would require coordinated updates.

| Constant | Value | Defined in | Used by |
|----------|-------|-----------|---------|
| `CS_P` | 64 | `greedy_mesher/src/core.rs` | Column bitmask width, opaque_mask stride |
| `CS` | 62 | `greedy_mesher/src/core.rs` | Usable chunk size, coordinate math |
| `CS_P2` | 4096 | `greedy_mesher/src/core.rs` | opaque_mask array length |
| `CS_P3` | 262144 | `greedy_mesher/src/core.rs` | Total padded voxels per chunk |
| `MATERIAL_EMPTY` | 0 | `greedy_mesher/src/core.rs` | Air/unoccupied sentinel |
| `MATERIAL_DEFAULT` | 1 | `greedy_mesher/src/core.rs` | Solid with no explicit material |
| `FACE_*` | 0-5 | `greedy_mesher/src/core.rs` | Face direction indices |
| Packed quad | 8 bytes | `greedy_mesher/src/core.rs` | Binary meshing intermediate |

---

## The Five Pillars

Reducing the architecture to its essence, there are five core pillars that
everything else builds on. These are the pieces of logic that, if they break
or change, affect the widest blast radius:

### Pillar 1: BinaryChunk — The Canonical Voxel Representation

`opaque_mask: [u64; 4096]` + `PaletteMaterials`

- Read by: greedy meshing (face culling, merge), radiance cascade raymarching,
  LOD point generation, debug visualization
- Written by: voxelizer ingestion, user edits, procedural generation
- Uploaded to GPU as: 3D texture (cascades), implicitly via MeshOutput (rendering)

**If this changes:** Every consumer must update. The u64 column layout is
baked into the meshing algorithm (ADR-0003) and the decision not to write
opaque_mask from the GPU (ADR-0009).

### Pillar 2: ChunkManager — The State Orchestrator

Dirty tracking → rebuild scheduling → mesh output → swap

- Coordinates all writes (voxelizer, user, procedural)
- Enforces version consistency (no stale mesh applied)
- Manages memory budget and eviction
- Provides the "single source of truth" principle (`voxelizer-integration/philosophy.md`)

**If this changes:** The frame update loop, worker protocol, and all data
upload paths must update.

### Pillar 3: MeshOutput — The Geometry Contract

`positions: Vec<f32>, normals: Vec<f32>, indices: Vec<u32>, uvs: Vec<f32>, material_ids: Vec<u16>`

- Produced by: greedy mesher
- Consumed by: buffer pool (rendering), Three.js mesh builder (current),
  cluster metadata generator (future)
- Extended by: `ClusterOffset` metadata (ADR-0011 Stage 2+)

**If this changes:** WASM bindings, worker transfer, buffer upload, and
shader vertex layout must all update.

### Pillar 4: Depth Texture — The GPU Shared Resource

App-owned `GPUTexture` with depth prepass output.

- Read by: Hi-Z pyramid build, radiance cascade probe placement, two-phase
  occlusion culling, main color pass (depth test EQUAL)
- Written by: depth prepass (all visible chunk meshes)

**If this doesn't exist:** Hi-Z culling, radiance cascades, and the custom
color pass are all blocked. This is the #1 prerequisite.

### Pillar 5: MaterialId + MaterialDef — The Material Pipeline

`u16` ID → `{color, roughness, metalness, emissive, texture}` properties

- Written by: voxelizer (material_table resolution), user edits
- Stored in: PaletteMaterials (per-chunk compressed), MaterialRegistry (TS)
- Consumed by: fragment shader (atlas lookup + PBR), radiance cascades
  (emissive = light sources), debug visualization (color modes)

**If this changes:** Palette compression, WASM bindings, material upload,
shader material data texture, and cascade emissive lookup must all update.

---

## Implementation Priority

Based on the shared dependency analysis, the highest-leverage work items are:

| Priority | Item | Unblocks | Documented in |
|----------|------|----------|---------------|
| **P0** | Shared GPUDevice handle | All GPU pipeline work | `hybrid-transition.md` Phase 0 |
| **P1** | App-owned depth texture + prepass | Hi-Z, cascades, custom color | `hybrid-transition.md` Phase 1 |
| **P2** | Voxelizer→chunk ingestion (ADR-0009) | Full OBJ→render pipeline | `voxelizer-integration/` |
| **P3** | Occupancy 3D texture upload | Radiance cascades | `adr/0010` Phase 2 |
| **P4** | Chunk AABB buffer | Hi-Z culling | `frame-graph.md` Pass 3 |
| **P5** | Global mesh buffer pool | Indirect draw, custom color pass | `pipeline-architecture.md` |
| **P6** | Hi-Z pyramid + cull compute | Occlusion culling | `frame-graph.md` Pass 2-3 |
| **P7** | Cascade build + merge | Global illumination | `adr/0010` Phase 2-3 |
| **P8** | Custom color pass with GI | Lit rendering | `hybrid-transition.md` Phase 3 |
| **P9** | Indirect draw integration | GPU-driven rendering | `hybrid-transition.md` Phase 4 |
| **P10** | Cluster metadata + cull | Fine-grained culling | `visibility-buffer.md` |

---

## See Also

- [`legacy/greedy-meshing-docs/INDEX.md`](legacy/greedy-meshing-docs/INDEX.md) — meshing documentation hub
- [`voxelizer-integration/INDEX.md`](voxelizer-integration/INDEX.md) — voxelizer integration hub
- [`gpu-driven-rendering/INDEX.md`](gpu-driven-rendering/INDEX.md) — rendering pipeline hub
- [`culling/hiz-occlusion-culling-report.md`](culling/hiz-occlusion-culling-report.md) — culling readiness report
