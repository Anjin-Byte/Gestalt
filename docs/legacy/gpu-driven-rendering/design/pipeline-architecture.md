# Target Frame Pipeline Architecture

**Type:** legacy
**Status:** legacy
**Date:** 2026-03-09

---

## Overview

This document defines the per-frame GPU command structure for the custom voxel
chunk rendering pipeline. The pipeline replaces Three.js's `renderer.render()`
call for chunk geometry only. Non-chunk objects (debug helpers, UI overlays)
continue to render through Three.js as a final overlay pass.

---

## Frame Pipeline

```
┌─────────────────────────────────────────────────────────────────┐
│ CPU: Update camera uniforms, upload dirty chunk data            │
└─────────────────────┬───────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────────────┐
│ Pass 1: DEPTH PREPASS (render pass)                             │
│                                                                 │
│   Input:  chunk mesh index/vertex buffers, camera MVP           │
│   Output: depth_texture (app-owned, RENDER_ATTACHMENT |         │
│           TEXTURE_BINDING)                                      │
│   Draw:   indirect indexed draw from chunk draw args buffer     │
│   Note:   all chunks drawn (no culling yet on first frame;      │
│           subsequent frames use previous frame's cull result)   │
└─────────────────────┬───────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────────────┐
│ Pass 2: HI-Z PYRAMID BUILD (compute)                           │
│                                                                 │
│   Input:  depth_texture (mip 0)                                 │
│   Output: depth_pyramid (mip 1..N, conservative min/max)        │
│   Dispatch: one dispatch per mip level, sequential              │
│   Ref: docs/culling/hiz-occlusion-culling-report.md §4          │
└─────────────────────┬───────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────────────┐
│ Pass 3: OCCLUSION CULL (compute)                                │
│                                                                 │
│   Input:  depth_pyramid, chunk_bounds[] (AABB center+extents),  │
│           camera matrices, cluster_bounds[] (if meshlets)       │
│   Output: visibility_buffer (u32 bitfield or indirect args)     │
│   Per-unit: project AABB to screen, sample pyramid at matching  │
│           mip, compare max-depth of AABB against pyramid min    │
│   Ref: docs/culling/hiz-occlusion-culling-report.md §10-11     │
└─────────────────────┬───────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────────────┐
│ Pass 4: RADIANCE CASCADE BUILD (compute) — if GI enabled        │
│                                                                 │
│   Input:  depth_texture, occupancy_3d_texture, emissive_data,   │
│           previous_frame_cascades (temporal)                    │
│   Output: cascade_atlas (RGBA16F, all N cascades packed)        │
│                                                                 │
│   4a: For cascade N-1 down to 0:                                │
│       - Reconstruct world pos from depth for each probe         │
│       - Raymarch occupancy for interval [2^i, 2^(i+1)]          │
│       - Store radiance + transparency per direction             │
│                                                                 │
│   4b: Temporal blend with reprojected previous cascades         │
│                                                                 │
│   4c: Merge cascades back-to-front (Eq. 13 of Sannikov):       │
│       L_merged = L_i + β_i × L_(i+1)                           │
│                                                                 │
│   Ref: docs/adr/0010-radiance-cascades.md                        │
└─────────────────────┬───────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────────────┐
│ Pass 5: MAIN COLOR PASS (render pass)                           │
│                                                                 │
│   Input:  chunk mesh buffers, material_atlas, material_data,    │
│           merged cascade_atlas (GI), camera, lights             │
│   Output: color_texture, depth_texture (reused from pass 1)     │
│   Draw:   indirect indexed draw (visibility-filtered)           │
│                                                                 │
│   Fragment shader:                                              │
│     1. Sample material atlas for albedo (ADR-0007)              │
│     2. Read material properties (roughness, metalness)          │
│     3. Direct lighting (analytic lights)                        │
│     4. Query cascade_atlas for indirect diffuse + specular      │
│     5. Combine: final = direct + indirect_diffuse + specular    │
│                                                                 │
│   Depth test: EQUAL (reuse prepass depth, no redundant writes)  │
└─────────────────────┬───────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────────────┐
│ Pass 6: THREE.JS OVERLAY (render pass — existing renderer)      │
│                                                                 │
│   Input:  Three.js scene (helpers, debug viz, sprites)          │
│   Output: composited onto color_texture                         │
│   Method: renderer.render(overlayScene, camera)                 │
│   Note:   renders to same canvas; depth test against pass 5     │
│           depth so overlays occlude correctly                   │
└─────────────────────────────────────────────────────────────────┘
```

---

## Resource Table

| Resource | Format | Size (1080p) | Created by | Read by | Lifetime |
|----------|--------|-------------|-----------|---------|----------|
| `depth_texture` | `depth24plus` | ~8 MB | Pass 1 | Pass 2, 4, 5 | Per-frame (double-buffered) |
| `depth_pyramid` | `r32float` mips | ~11 MB | Pass 2 | Pass 3 | Per-frame |
| `chunk_bounds` | `vec4f` × 2 per chunk | ~32 KB (4096 chunks) | CPU upload | Pass 3 | Persistent, update on dirty |
| `visibility_buffer` | `u32[]` or indirect args | ~16 KB (4096 chunks) | Pass 3 | Pass 5 | Per-frame |
| `occupancy_3d_texture` | `r32uint` | ~32 MB (256³ world) | CPU upload (dirty chunks) | Pass 4 | Persistent, incremental update |
| `emissive_data` | Material data texture | ~64 KB | CPU upload | Pass 4 | Persistent |
| `cascade_atlas` | `rgba16f` | ~64 MB (4 cascades) | Pass 4 | Pass 5 | Double-buffered (temporal) |
| `cascade_atlas_prev` | `rgba16f` | ~64 MB | Previous frame | Pass 4 | Swap each frame |
| `color_texture` | `rgba8unorm` | ~8 MB | Pass 5 | Canvas / Pass 6 | Per-frame |
| `chunk_mesh_buffer` | Vertex + index data | Variable | CPU upload | Pass 1, 5 | Persistent per chunk |
| `indirect_args_buffer` | `DrawIndexedIndirectArgs[]` | ~80 KB (4096 chunks) | Pass 3 (GPU) | Pass 1*, 5 | Per-frame |

*Pass 1 uses previous frame's cull result on subsequent frames (two-phase occlusion culling).

---

## Data Upload Strategy

### Chunk Mesh Data

Greedy-meshed chunk geometry is uploaded to a shared vertex/index buffer pool.
Each chunk occupies a contiguous range in the pool. The pool is structured as:

```
Global Vertex Buffer:  [chunk_0 vertices | chunk_1 vertices | ... | chunk_N vertices]
Global Index Buffer:   [chunk_0 indices  | chunk_1 indices  | ... | chunk_N indices ]
Draw Args Buffer:      [DrawArgs_0       | DrawArgs_1       | ... | DrawArgs_N      ]
```

When a chunk mesh is rebuilt, its slot in the pool is updated. The draw args
buffer stores per-chunk `DrawIndexedIndirectArgs` with the correct offsets
into the global buffers.

This replaces the current `ChunkMeshPool` pattern (one `THREE.Mesh` per chunk)
with a single buffer pool and a single multi-draw-indirect call.

### Chunk Occupancy Data (for Radiance Cascades)

The `opaque_mask` data from all loaded chunks is packed into a 3D texture.
Updates are incremental: when a chunk becomes dirty and is re-meshed, only
its region in the 3D texture is updated via `writeTexture`.

The 3D texture is addressed as `(world_vx, world_vy, world_vz)` with the
chunk manager's origin as the texture origin. Chunks outside the loaded range
sample as empty (transparent).

### Chunk Bounds Data (for Culling)

Per-chunk AABB (center + extents) is stored in a structured buffer. Updated
when chunks are loaded/unloaded or when mesh bounds change after rebuild.

---

## Culling Granularity Progression

The pipeline is designed for increasing culling granularity over time:

### Stage 1: Chunk-Level Culling (MVP)

- One cull unit per chunk (64³ voxels)
- One `DrawIndexedIndirectArgs` per chunk
- Visibility buffer: 1 bit per chunk (4096 chunks = 512 bytes)
- Sufficient for large occluders (walls, terrain, buildings)

### Stage 2: Face-Direction Splitting

- 6 draw ranges per chunk (±X, ±Y, ±Z)
- The greedy mesher already processes faces per axis; output them as 6
  contiguous sub-ranges in the index buffer
- Backface directions can be trivially culled (dot product with view) before
  occlusion test — eliminates 3 of 6 sub-ranges per chunk immediately
- `DrawIndexedIndirectArgs` count: 6 × 4096 = 24,576 (still cheap)

### Stage 3: Meshlet/Cluster Culling

- Sub-divide each face-direction group into clusters of 64-128 triangles
- Each cluster has its own AABB
- Cluster culling via Hi-Z test per cluster
- Enables fine-grained rejection inside large chunks (e.g., interior rooms
  occluding back walls)
- Requires a visibility buffer approach or multi-draw-indirect

### Stage 4: Visibility Buffer Rendering (Optional, Future)

- Replace traditional vertex shading with a visibility buffer
- Pass 1 writes (triangle_id, cluster_id) per pixel instead of shaded color
- Pass 5 becomes a fullscreen compute/fragment that resolves materials
- Eliminates overdraw entirely (each pixel shaded exactly once)
- Significant architectural change; see `spec/visibility-buffer.md`

---

## Performance Characteristics

### Pass Timing Budget (Target: 60 FPS = 16.6ms)

| Pass | Budget | Notes |
|------|--------|-------|
| CPU: upload + uniform update | 0.5-1ms | Dirty chunk data only |
| Pass 1: Depth prepass | 0.5-1ms | Depth-only, no fragment shading |
| Pass 2: Hi-Z pyramid | 0.2-0.5ms | Log2(resolution) dispatches |
| Pass 3: Occlusion cull | 0.1-0.3ms | One thread per cull unit |
| Pass 4: Radiance cascades | 3-6ms | With temporal amortization |
| Pass 5: Main color pass | 2-4ms | Visibility-filtered, GI in fragment |
| Pass 6: Three.js overlay | 0.5-1ms | Minimal geometry (helpers) |
| **Total** | **7-14ms** | Within 16.6ms budget |

### Without Radiance Cascades (GI Disabled)

| Pass | Budget |
|------|--------|
| CPU + Pass 1-3 | 1.3-2.8ms |
| Pass 5 (no GI) | 1-2ms |
| Pass 6 | 0.5-1ms |
| **Total** | **2.8-5.8ms** |

This leaves significant headroom for other work (meshing, voxelization, UI).

---

## Failure Modes and Fallbacks

| Condition | Behavior |
|-----------|----------|
| WebGPU unavailable | Fall back to Three.js-only rendering (WebGL2). No culling, no GI. Current behavior preserved. |
| Compute shader failure | Disable culling, render all chunks. Disable GI. Log warning. |
| Depth prepass stalls | Skip Hi-Z + cascades this frame, render with previous frame's data. |
| Memory budget exceeded | Reduce cascade count, disable temporal history, coarsen pyramid. |
| Debug mode | Bypass indirect draw, render all chunks directly for inspection. |

---

## See Also

- [`three-js-limits.md`](three-js-limits.md) — why Three.js can't host this pipeline
- [`hybrid-transition.md`](hybrid-transition.md) — incremental migration plan
- [`../spec/frame-graph.md`](../spec/frame-graph.md) — formal pass ordering and resource dependencies
- [`../spec/visibility-buffer.md`](../spec/visibility-buffer.md) — meshlet and visibility buffer design
- [`../../../adr/0010-radiance-cascades.md`](../../../adr/0010-radiance-cascades.md) — Pass 4 design
- [`../../culling/hiz-occlusion-culling-report.md`](../../culling/hiz-occlusion-culling-report.md) — Pass 2-3 design
