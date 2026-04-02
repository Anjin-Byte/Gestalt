# Pipeline Stage Specifications

**Type:** reference
**Status:** current
**Date:** 2026-03-22

> One document per pipeline stage. Preconditions, inputs, transformation, outputs, postconditions, and how to test the stage in isolation.

Each document answers: What goes in? What comes out? What must be true before and after?

---

## Ingest Stages

| Stage | Doc | Trigger | Key output |
|---|---|---|---|
| I-1 | [I-1-voxelization](I-1-voxelization.md) | New mesh / procedural gen | `chunk_occupancy_atlas` entries |
| I-2 | [I-2-chunk-upload](I-2-chunk-upload.md) | After I-1 | GPU buffer populated |
| I-3 | [I-3-summary-rebuild](I-3-summary-rebuild.md) | After I-2 or edit | `occupancy_summary`, `chunk_flags`, `chunk_aabb` |

## Per-Frame Render Stages

| Stage | Doc | Type | Key output |
|---|---|---|---|
| R-1 | [R-1-mesh-rebuild](R-1-mesh-rebuild.md) | Compute | `vertex_pool`, `index_pool`, `draw_metadata` |
| R-2 | [R-2-depth-prepass](R-2-depth-prepass.md) | Render | `depth_texture` |
| R-3 | [R-3-hiz-build](R-3-hiz-build.md) | Compute | `hiz_pyramid` |
| R-4 | [R-4-occlusion-cull](R-4-occlusion-cull.md) | Compute | `indirect_draw_buf` |
| R-5 | [R-5-color-pass](R-5-color-pass.md) | Render | `color_target` |
| R-6 | [R-6-cascade-build](R-6-cascade-build.md) | Compute | `cascade_atlas` per level |
| R-7 | [R-7-cascade-merge](R-7-cascade-merge.md) | Compute | Merged `cascade_atlas_0` |
| R-8 | [R-8-gi-composite](R-8-gi-composite.md) | Inline (R-5 fragment) | Final color with GI |
| R-9 | [R-9-debug-viz](R-9-debug-viz.md) | Render/Compute | Debug overlay on `color_target` |
