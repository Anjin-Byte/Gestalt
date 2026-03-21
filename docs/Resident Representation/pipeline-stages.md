# GPU-First Pipeline Stage Diagram

Exact buffers, textures, and read/write ownership per stage.
Derived from [[layer-model]] — three products from one voxel truth.

---

## Stage Overview

```
 ┌──────────────────────────────────────────────────────────────────┐
 │ INGEST (one-time or on edit)                                     │
 │  Stage I-1: Voxelization                                         │
 │  Stage I-2: Chunk Occupancy Upload                               │
 │  Stage I-3: Derived Summary Rebuild                              │
 └──────────────────────────────────────────────────────────────────┘
                          │
                          ▼
 ┌──────────────────────────────────────────────────────────────────┐
 │ PER-FRAME RENDER                                                 │
 │  Stage R-1: Greedy Mesh Rebuild (dirty chunks only)             │
 │  Stage R-2: Depth Prepass                                        │
 │  Stage R-3: Hi-Z Pyramid Build                                   │
 │  Stage R-4: Occlusion Cull + Indirect Arg Write                  │
 │  Stage R-5: Main Color Pass                                      │
 │  Stage R-6: Radiance Cascade Build                               │
 │  Stage R-7: Cascade Merge                                        │
 │  Stage R-8: GI Application + Composite                          │
 │  Stage R-9: Three.js Overlay                                     │
 └──────────────────────────────────────────────────────────────────┘
```

---

## Ingest Stages

### Stage I-1: Voxelization
*Trigger: new OBJ load, procedural generation, or bulk edit*

| Buffer / Texture | Direction | Format | Notes |
|---|---|---|---|
| `mesh_positions` | READ | `array<f32>` | Input triangle positions |
| `mesh_indices` | READ | `array<u32>` | Input triangle indices |
| `material_table` | READ | `array<u32>` | Packed u16×2 per u32 |
| `brick_origins_buf` | READ | `array<u32×4>` | CSR brick world coords |
| `brick_offsets_buf` | READ | `array<u32>` | CSR row pointers |
| `tri_indices_buf` | READ | `array<u32>` | CSR triangle lists |
| `occupancy_scratch` | WRITE | `array<u32>` | Bitpacked, per-brick |
| `owner_scratch` | WRITE | `array<u32>` | Triangle ID per voxel |

Owner: `crates/voxelizer` GPU compute. All buffers are **transient** — allocated for this dispatch, discarded afterward.

Output: `CompactVoxel[]` (CPU-side, emitted by `compact_surface_sparse`). This is the courier into Stage I-2.

---

### Stage I-2: Chunk Occupancy Upload
*Trigger: after voxelization, or on CPU-side edit*

Reads `CompactVoxel[]` on CPU. For each voxel, writes into the GPU chunk pool:

| Buffer / Texture | Direction | Format | Notes |
|---|---|---|---|
| `chunk_occupancy_atlas` | WRITE | `r32uint` 3D texture or `array<u32>` per slot | 2048 u32s per chunk (64³ / 32 bits) |
| `chunk_palette_buf` | WRITE | `array<u32>` per slot | Palette material IDs |
| `chunk_index_buf` | WRITE | `array<u32>` per slot | Bitpacked palette indices per voxel |
| `chunk_slot_table` | WRITE | CPU `HashMap<ChunkCoord, SlotIndex>` | CPU-only slot directory |
| `chunk_resident_flags` | WRITE | `array<u32>` | CPU sets `is_resident = 1` on load |

Owner: CPU (Web Worker, Rust ChunkManager). Upload via `writeBuffer` or mapped staging buffer.

After upload, CPU sets `chunk_resident_flags[slot].is_resident = 1` and sets the `stale_summary` control-plane bit for this slot. The GPU compaction pass then enqueues the slot into `summary_rebuild_queue` (see [[edit-protocol]] — CPU must not write queues directly).

---

### Stage I-3: Derived Summary Rebuild
*Trigger: after occupancy upload for any dirty chunk*

Compute shader reads `chunk_occupancy_atlas`, writes summaries.

| Buffer / Texture | Direction | Format | Notes |
|---|---|---|---|
| `chunk_occupancy_atlas` | READ | see above | Per-chunk occupancy |
| `chunk_palette_buf` | READ | see above | For emissive flag |
| `chunk_flags` | WRITE | `array<u32>` | One u32 per slot: `is_empty`, `has_emissive` |
| `occupancy_summary` | WRITE | `array<u32>` per slot | Coarse bricklet grid (8³ bits) |
| `chunk_aabb` | WRITE | `array<vec4f×2>` | Tight world-space bounds per slot |

Owner: GPU compute. Workgroup per dirty chunk. Runs once per dirty chunk after upload.

After this stage, the `stale_summary` control-plane bit for the chunk is cleared and `summary_version[slot]` is stamped.

---

## Per-Frame Render Stages

### Stage R-1: Greedy Mesh Rebuild
*Trigger: chunks dirtied since last frame*

Not a GPU compute stage currently — runs on CPU (Rust, Web Worker). Reads `opaque_mask` from CPU-mirrored chunk data, emits `ChunkMeshTransfer`.

Future target: GPU compute reading `chunk_occupancy_atlas`, writing into `vertex_pool` and `index_pool` directly. See [[gpu-chunk-pool]].

| Buffer | Direction | Format | Notes |
|---|---|---|---|
| CPU `opaque_mask` | READ | `[u64; 4096]` | CPU-mirrored |
| CPU `materials` | READ | `PaletteMaterials` | CPU-mirrored |
| `vertex_pool` | WRITE | `array<f32>` | Positions + normals, all chunks |
| `index_pool` | WRITE | `array<u32>` | Indices, all chunks |
| `draw_metadata` | WRITE | `array<DrawMetadata>` | Per-chunk: slot, vertex range, index range, coord |

---

### Stage R-2: Depth Prepass
*Every frame*

Renders chunk geometry depth-only. No color output.

| Buffer / Texture | Direction | Format | Notes |
|---|---|---|---|
| `vertex_pool` | READ | `array<f32>` | Chunk vertices |
| `index_pool` | READ | `array<u32>` | Chunk indices |
| `draw_metadata` | READ | `array<DrawMetadata>` | Per-chunk draw ranges |
| `camera_uniform` | READ | `mat4×2` | View + projection |
| `depth_texture` | WRITE | `depth32float` | **App-owned.** Shared with R-3, R-6 |

Owner: custom WebGPU render pipeline. Prerequisite for all downstream stages.

---

### Stage R-3: Hi-Z Pyramid Build
*Every frame, after R-2*

Compute shader reads `depth_texture`, builds full mip chain for occlusion testing.

| Buffer / Texture | Direction | Format | Notes |
|---|---|---|---|
| `depth_texture` | READ | `depth32float` | From R-2 |
| `hiz_pyramid` | WRITE | `r32float` mipped 2D texture | Full mip chain, max-depth per cell |

Owner: GPU compute. One dispatch per mip level (or single pass with shared memory).

---

### Stage R-4: Occlusion Cull + Indirect Arg Write
*Every frame, after R-3*

Two-phase dispatch. Phase 1 culls at chunk AABB granularity and writes a visible-chunk list.
Phase 2 culls at meshlet AABB granularity (indirect dispatch over the visible-chunk list) and
writes the final indirect draw arguments. See [[meshlets]] for full pseudocode and fallback behavior.

**Phase 1 — Chunk Coarse Cull**

| Buffer / Texture | Direction | Format | Notes |
|---|---|---|---|
| `chunk_aabb` | READ | `array<vec4f×2>` | Tight per-chunk bounds |
| `chunk_flags` | READ | `array<u32>` | Skip empty chunks early |
| `chunk_resident_flags` | READ | `array<u32>` | Skip non-resident slots |
| `hiz_pyramid` | READ | `r32float` mipped | From R-3 |
| `camera_uniform` | READ | `mat4×2` | View + projection |
| `chunk_visible_list` | WRITE | `array<u32>` | Surviving slot indices |
| `chunk_visible_count` | WRITE | `u32` | Atomic counter; sizes phase 2 indirect dispatch |

**Phase 2 — Meshlet Fine Cull** *(indirect dispatch over `chunk_visible_list`)*

| Buffer / Texture | Direction | Format | Notes |
|---|---|---|---|
| `chunk_visible_list` | READ | `array<u32>` | From phase 1 |
| `meshlet_range_table` | READ | `array<MeshletRange>` | Per-slot meshlet range |
| `meshlet_desc_pool` | READ | `array<MeshletDesc>` | Meshlet AABB, index offset, vertex base |
| `meshlet_version` | READ | `array<u32>` | Freshness check per slot |
| `chunk_version` | READ | `array<u32>` | Ground-truth version |
| `draw_metadata` | READ | `array<DrawMetadata>` | Fallback chunk-level draw when meshlets stale |
| `hiz_pyramid` | READ | `r32float` mipped | Reused from R-3 |
| `camera_uniform` | READ | `mat4×2` | View + projection |
| `indirect_draw_buf` | WRITE | `array<DrawIndexedIndirectArgs>` | One entry per passing meshlet (or chunk fallback) |
| `visible_meshlet_count` | WRITE | `u32` | Diagnostic atomic counter |

Owner: GPU compute. Output feeds R-5.

---

### Stage R-5: Main Color Pass
*Every frame, after R-4*

Renders visible chunk geometry to color target. Reads GI result from cascade merge (R-7 from previous frame or current frame if cascades run before color).

| Buffer / Texture | Direction | Format | Notes |
|---|---|---|---|
| `vertex_pool` | READ | `array<f32>` | |
| `index_pool` | READ | `array<u32>` | |
| `indirect_draw_buf` | READ | indirect | From R-4 |
| `depth_texture` | READ/TEST | `depth32float` | Depth test, no write |
| `material_table` | READ | `array<MaterialEntry>` | Global material properties; indexed by `vertex_material_id` emitted per quad by greedy mesher |
| `cascade_atlas_0` | READ | `rgba16float` 2D | Merged cascade 0 (from R-7) |
| `color_target` | WRITE | `rgba8unorm` | Final color output |

Fragment shader integrates hemisphere from cascade 0 for diffuse GI, cone query for specular.

---

### Stage R-6: Radiance Cascade Build
*Every frame (or amortized via temporal reprojection)*

For each probe (depth-buffer surface position), marches rays through world-space chunk occupancy.

| Buffer / Texture | Direction | Format | Notes |
|---|---|---|---|
| `depth_texture` | READ | `depth32float` | Probe world-position reconstruction |
| `camera_uniform` | READ | `mat4×2` | Unproject depth → world pos |
| `chunk_occupancy_atlas` | READ | `r32uint` 3D texture or buffer | **Product 1 input** |
| `chunk_flags` | READ | `array<u32>` | Skip empty chunks in traversal |
| `occupancy_summary` | READ | `array<u32>` | Skip empty bricklets |
| `chunk_palette_buf` | READ | per-slot | Per-voxel palette index → MaterialId lookup |
| `palette_meta` | READ | `array<u32>` per slot | `palette_size` + `bits_per_entry` for index unpack |
| `material_table` | READ | `array<MaterialEntry>` | Emissive property lookup after palette resolution |
| `chunk_slot_table_gpu` | READ | `array<ChunkSlotEntry>` | World coord → slot index |
| `cascade_atlas_N` | WRITE | `rgba16float` 2D | One atlas per cascade level |
| `cascade_atlas_prev` | READ | `rgba16float` 2D | Previous frame (temporal blend) |

Traversal follows Product 1 three-level DDA: chunk → sub-brick → voxel.
Each probe ray marches only its cascade interval `[t_i, t_{i+1}]` voxel units, where `t_0 = 0`, `t_1 = 1`, `t_2 = 2`, `t_3 = 4`, … (cascade 0 starts at 0, not 1).

Owner: GPU compute. One workgroup per probe tile.

---

### Stage R-7: Cascade Merge
*Every frame, after R-6*

Back-to-front merge (cascade N-1 → 0). Each cascade blends with the next via bilateral interpolation (depth-aware).

| Buffer / Texture | Direction | Format | Notes |
|---|---|---|---|
| `depth_texture` | READ | `depth32float` | Bilateral depth weights |
| `cascade_atlas_N` | READ | `rgba16float` | Higher cascade |
| `cascade_atlas_N-1` | READ/WRITE | `rgba16float` | Lower cascade (in-place merge) |
| `cascade_atlas_0` | WRITE | `rgba16float` | Final merged result for R-5 |

Owner: GPU compute. One pass per cascade level pair, back to front.

---

### Stage R-8: GI Application + Composite
*Handled inline in R-5 fragment shader or as a separate fullscreen pass*

If inline: fragment shader reads `cascade_atlas_0` and integrates.
If separate: fullscreen quad reads `cascade_atlas_0` + `color_target`, writes composited output.

---

### Stage R-9: Three.js Overlay
*Every frame, last*

Three.js renders debug helpers, grid, axes, sprites, UI geometry as an overlay.

| Buffer / Texture | Direction | Format | Notes |
|---|---|---|---|
| `depth_texture` | READ/TEST | `depth32float` | Depth test against chunk geometry |
| `color_target` | READ/WRITE | `rgba8unorm` | Composite over existing color |

Three.js does not render chunk geometry. It reads the same depth buffer to correctly composite debug overlays against existing chunk depth.

---

## Buffer Ownership Summary

| Buffer / Texture | Owner | Lifetime | Product |
|---|---|---|---|
| `depth_texture` | App (custom pipeline) | Per frame | 2, 3 |
| `hiz_pyramid` | App | Per frame | 3 |
| `chunk_occupancy_atlas` | Chunk pool | Scene lifetime | 1, 2 |
| `chunk_resident_flags` | Chunk pool | Scene lifetime | 1, 2, 3 |
| `chunk_flags` | Chunk pool | Scene lifetime | 1, 2, 3 |
| `occupancy_summary` | Chunk pool | Scene lifetime | 1 |
| `chunk_aabb` | Chunk pool | Scene lifetime | 3 |
| `chunk_palette_buf` | Chunk pool | Scene lifetime | 1, 2 |
| `palette_meta` | Chunk pool | Scene lifetime | 1, 2 |
| `material_table` | Material system | Scene lifetime | 1, 2 |
| `chunk_slot_table_gpu` | Chunk pool | Scene lifetime | 1 |
| `vertex_pool` | Chunk pool | Scene lifetime | 2 |
| `index_pool` | Chunk pool | Scene lifetime | 2 |
| `draw_metadata` | Chunk pool | Scene lifetime | 2, 3 |
| `meshlet_desc_pool` | Meshlet pool | Scene lifetime | 3 |
| `meshlet_range_table` | Meshlet pool | Scene lifetime | 3 |
| `chunk_visible_list` | Cull pass | Per frame | 3 |
| `indirect_draw_buf` | Cull pass | Per frame | 3 |
| `cascade_atlas_*` | RC system | Per frame + prev | 1 (queries) |
| `camera_uniform` | App | Per frame | all |

---

## Read/Write Conflict Rules

1. `depth_texture` is written by R-2 only. All later stages read it. No concurrent writes.
2. `chunk_occupancy_atlas` is written during ingest (I-2), read during R-6. Never written during render.
3. `cascade_atlas_0` is written by R-7, read by R-5. R-5 must not begin until R-7 completes (explicit barrier or separate submit).
4. `indirect_draw_buf` is written by R-4, consumed by R-5 indirect draw. Pipeline barrier between compute and indirect draw.
5. Three.js (R-9) must not clear `depth_texture` — it must be configured to reuse the existing depth attachment.

---

## See Also

- [[chunk-contract]] — canonical chunk field specification
- [[layer-model]] — three-product architecture and the canonical source
- [[traversal-acceleration]] — three-level DDA design for Stage R-6
- [[gpu-chunk-pool]] — slot allocation, atlas layout, residency management
