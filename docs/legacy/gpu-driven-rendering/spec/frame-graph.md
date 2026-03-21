# Frame Graph — Pass Ordering and Resource Dependencies

**Type:** legacy
**Status:** legacy
**Date:** 2026-03-09

---

## Purpose

This document defines the formal ordering of GPU passes within a single frame,
the resource dependencies between them, and the synchronization requirements.
It serves as the implementation contract for the custom rendering pipeline.

---

## Pass Definitions

### Pass 1: Depth Prepass

```
Type:       Render pass (depth-only)
Pipeline:   Vertex shader only, no fragment output
Input:      chunk_vertex_buffer, chunk_index_buffer, camera_ubo
Output:     depth_texture (write)
Draw mode:  drawIndexedIndirect(indirect_args_buffer)  [Phase 4+]
            drawIndexed per chunk                       [Phase 1-3]
Clear:      depth = 1.0
```

**Note:** On the first frame, all chunks are drawn (no prior cull data). On
subsequent frames, the indirect args buffer contains the previous frame's cull
result (two-phase occlusion culling). Newly loaded chunks are always drawn.

### Pass 2: Hi-Z Pyramid Build

```
Type:       Compute (sequential dispatches)
Pipeline:   Downsample shader (conservative min for reversed-Z, max for standard)
Input:      depth_texture mip 0
Output:     depth_pyramid mip 1..N
Dispatches: ceil(log2(max(width, height))) dispatches
            Each reads mip i-1, writes mip i
Workgroup:  8×8 (each thread reads 2×2 texels, writes 1 texel)
```

### Pass 3: Occlusion Cull

```
Type:       Compute (single dispatch)
Pipeline:   Cull shader
Input:      depth_pyramid, chunk_bounds_buffer, camera_ubo
Output:     indirect_args_buffer (write instance_count = 0 or 1 per chunk)
            Optional: visibility_bitfield (for debug readback)
Dispatch:   ceil(chunk_count / 64) workgroups × 1 × 1
Workgroup:  64 threads (one per chunk)
```

**Per-thread logic:**
1. Read chunk AABB (center, extents) from `chunk_bounds_buffer`
2. Project AABB to screen-space bounding rect
3. Determine pyramid mip level from rect size
4. Sample pyramid at bounding rect corners
5. Compare chunk's max depth against pyramid's min depth
6. If occluded: `indirect_args[chunk_id].instance_count = 0`
7. If visible: `indirect_args[chunk_id].instance_count = 1`

### Pass 4: Radiance Cascade Build (Optional)

```
Type:       Compute (multiple dispatches)
Pipeline:   Cascade raymarch shader, cascade merge shader

Sub-pass 4a — Raymarch (per cascade, highest to lowest):
  Input:    depth_texture, occupancy_3d_texture, emissive_data_texture,
            camera_ubo, cascade_params_ubo
  Output:   cascade_atlas (region for cascade i)
  Dispatch: ceil(probe_count_x / 8) × ceil(probe_count_y / 8) × 1
  Workgroup: 8×8 (one thread per probe)

Sub-pass 4b — Temporal blend:
  Input:    cascade_atlas (current), cascade_atlas_prev (previous frame),
            camera_motion_ubo
  Output:   cascade_atlas (blended in-place)
  Dispatch: same as 4a

Sub-pass 4c — Merge (back-to-front, cascade N-1 down to 0):
  Input:    cascade_atlas (cascades i and i+1)
  Output:   cascade_atlas (cascade i, merged in-place)
  Dispatch: ceil(probe_count_x_i / 8) × ceil(probe_count_y_i / 8) × 1
```

### Pass 5: Main Color Pass

```
Type:       Render pass (color + depth)
Pipeline:   Vertex + fragment shader
Input:      chunk_vertex_buffer, chunk_index_buffer, camera_ubo,
            material_atlas_texture, material_data_texture,
            cascade_atlas (merged), light_ubo
Output:     color_texture (write), depth_texture (test EQUAL, no write)
Draw mode:  drawIndexedIndirect(indirect_args_buffer)  [Phase 4+]
            drawIndexed per chunk                       [Phase 1-3]
Clear:      color = background_color
Depth:      depthCompare: equal, depthWriteEnabled: false
```

### Pass 6: Three.js Overlay

```
Type:       Three.js renderer.render() call
Input:      overlay_scene (helpers, debug objects, sprites)
Output:     canvas (composited on top of pass 5 output)
Depth:      Reads depth from pass 5 for correct occlusion
```

---

## Resource Dependency Graph

```
                    chunk_vertex_buffer ──────────────────────────────┐
                    chunk_index_buffer ───────────────────────────────┤
                    camera_ubo ──────────────────────────────────────┤
                                                                     │
                    ┌─────────────┐                                  │
                    │  Pass 1     │◀─────────────────────────────────┘
                    │  Depth      │◀── indirect_args_buffer (prev frame)
                    │  Prepass    │
                    └──────┬──────┘
                           │
                    depth_texture
                           │
              ┌────────────┼────────────┐
              ▼            │            ▼
       ┌──────────┐        │     ┌──────────────┐
       │  Pass 2  │        │     │  Pass 4      │
       │  Hi-Z    │        │     │  Cascades    │◀── occupancy_3d_texture
       │  Pyramid │        │     │              │◀── emissive_data_texture
       └────┬─────┘        │     │              │◀── cascade_atlas_prev
            │              │     └──────┬───────┘
     depth_pyramid         │            │
            │              │     cascade_atlas
            ▼              │            │
       ┌──────────┐        │            │
       │  Pass 3  │        │            │
       │  Cull    │◀── chunk_bounds     │
       └────┬─────┘        │            │
            │              │            │
    indirect_args_buffer   │            │
            │              │            │
            ▼              ▼            ▼
       ┌─────────────────────────────────────┐
       │  Pass 5: Main Color Pass            │
       │                                     │◀── material_atlas_texture
       │                                     │◀── material_data_texture
       │                                     │◀── light_ubo
       └──────────────┬──────────────────────┘
                      │
               color_texture + depth_texture
                      │
                      ▼
               ┌──────────────┐
               │  Pass 6      │
               │  Three.js    │
               │  Overlay     │
               └──────────────┘
```

---

## Synchronization

WebGPU command encoders execute passes in submission order. Within a single
`commandEncoder`, passes are sequential. Resource transitions are implicit
in WebGPU (no explicit barriers needed like in Vulkan/D3D12).

**Requirements:**
1. Passes 1-5 must be encoded on the **same command encoder** to guarantee
   ordering without explicit synchronization.
2. Pass 6 (Three.js) submits its own command encoder. It must be submitted
   **after** the custom pipeline's encoder.
3. `cascade_atlas_prev` is the previous frame's `cascade_atlas`. The swap
   happens at the end of the frame (after pass 4 completes).

**Double-buffered resources:**
- `cascade_atlas` / `cascade_atlas_prev` — swapped each frame
- `indirect_args_buffer` — written by pass 3, read by pass 1 of the **next**
  frame (two-phase occlusion). Must not be overwritten before pass 1 reads it.
  Solution: double-buffer the indirect args, or delay pass 3 write until after
  pass 1 read.

---

## Conditional Pass Execution

| Condition | Passes executed |
|-----------|----------------|
| GI enabled, culling enabled | 1, 2, 3, 4, 5, 6 |
| GI enabled, culling disabled | 1, 4, 5, 6 |
| GI disabled, culling enabled | 1, 2, 3, 5, 6 |
| GI disabled, culling disabled | 1, 5, 6 (or skip 1 if no consumers) |
| WebGL2 fallback | 6 only (Three.js renders everything) |
| Debug: depth viz | 1, 2, then fullscreen quad display |
| Debug: cascade viz | 1, 4, then cascade atlas display |

---

## See Also

- [`../design/pipeline-architecture.md`](../design/pipeline-architecture.md) — architectural overview and timing budgets
- [`../design/hybrid-transition.md`](../design/hybrid-transition.md) — phased rollout
- [`visibility-buffer.md`](visibility-buffer.md) — Stage 4 extension to this frame graph
