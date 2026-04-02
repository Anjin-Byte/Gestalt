# Test: Full Pipeline Consistency (Randomized Scene → Ingest → Render → Verify)

**Type:** spec
**Status:** current
**Date:** 2026-03-22

> The ultimate consistency test. Proves that a randomized scene survives full ingest and multi-frame rendering with no invariant violations, and that mid-scene edits propagate correctly through the dirty/stale/rebuild pipeline.

---

## What This Tests

The full pipeline spans every stage in the system:

```
Random scene generation
  → I-1 (voxelization) → I-2 (upload) → I-3 (summary)
  → N render frames: R-1 (mesh rebuild) → R-2 (cull) → R-3 (depth prepass)
    → R-4 (material) → R-5 (composite) → R-6..R-9 (lighting, post)
  → invariant checks after every frame
```

This is not a unit test. It is a property-based integration test that exercises the entire data flow with randomized inputs and verifies that no data structure invariant is ever violated, regardless of scene content, camera position, or edit timing.

---

## Baseline Test: Static Scene

### Setup

```
T-FP-1: Random static scene, full ingest, multi-frame render

  Scene generation:
    Generate N chunks where N ∈ [100, 300] (uniformly random)
    For each chunk:
      Assign random world coordinate (no duplicates)
      Generate random occupancy: each voxel has P(occupied) = random ∈ [0.05, 0.95] per chunk
      Assign 1-4 random palette entries with valid material IDs

  Ingest:
    Run I-1 (voxelization) for all chunks
    Run I-2 (upload) for all chunks — allocate pool slots, write occupancy + palette + index
    Run I-3 (summary rebuild) for all chunks

  Post-ingest verification (before any render):
    For each resident slot S:
      Assert: chunk_resident_flags[S] == 1
      Assert: chunk_version[S] > 0
      Assert: chunk_coord[S] is the assigned world coordinate
```

### Render Loop

```
  Render 10 frames with camera orbiting the scene centroid:
    Frame i: camera at angle (i * 36°), distance = scene_radius * 2

    After each frame, verify ALL of the following:
```

### Per-Frame Invariant Checks

#### Occupancy (OCC)

```
    For every resident slot S:
      OCC-1: chunk_occupancy_atlas[S] is exactly 8192 u32 words (32768 bytes)
      OCC-2: popcount(chunk_occupancy_atlas[S]) matches the expected voxel count from generation
```

#### Flags (FLG)

```
    For every resident slot S:
      FLG-1: chunk_flags.is_empty == (popcount(occupancy_atlas[S]) == 0)
      FLG-2: If is_empty == 0: chunk_flags.is_fully_opaque == (popcount == 64³ and all materials opaque)
      FLG-3: chunk_flags.has_emissive == 1 iff any palette entry references an emissive material
```

#### Summary (SUM)

```
    For every resident slot S with stale_summary == 0:
      SUM-1: For each bricklet b in [0, 511]:
        occupancy_summary bit b == 1 ⟺ bricklet b has at least one occupied voxel
      SUM-2: summary_version[S] == chunk_version[S]
```

#### AABB

```
    For every resident slot S with stale_summary == 0 and is_empty == 0:
      AABB-1: chunk_aabb.min < chunk_aabb.max (component-wise)
      AABB-2: Every occupied voxel's world position is inside chunk_aabb
      AABB-3: chunk_aabb is tight — min and max each touch at least one occupied voxel
```

#### Palette and Index (PAL, IDX)

```
    For every resident slot S:
      PAL-1: Every palette entry references a valid material ID in material_table
      IDX-1: Every per-voxel palette index is within [0, palette_size) for that slot
```

#### Version (VER)

```
    For every resident slot S:
      VER-1: chunk_version[S] > 0
      VER-2: mesh_version[S] <= chunk_version[S]
      VER-3: summary_version[S] <= chunk_version[S]
      VER-4: If stale_mesh[S] == 0: mesh_version[S] == chunk_version[S]
      VER-5: If stale_summary[S] == 0: summary_version[S] == chunk_version[S]
```

#### Materials (MAT)

```
    For every material ID referenced by any resident chunk palette:
      MAT-1: material_table entry exists
      MAT-2: Albedo values are in [0.0, 1.0] per channel
      MAT-3: Emissive values are non-negative
```

#### Draw Commands (DRW)

```
    For every resident slot S with is_empty == 0 and stale_mesh == 0:
      DRW-1: indirect_draw_buf entry for S has non-zero vertex count
      DRW-2: Vertex offset + vertex count does not exceed vertex pool capacity
      DRW-3: Index offset + index count does not exceed index pool capacity
    For every resident slot S with is_empty == 1:
      DRW-4: indirect_draw_buf entry for S has zero vertex count or is culled
```

#### GPU Output Sanity

```
    After each frame's render completes:
      GPU-1: depth_texture has no NaN values (readback and scan)
      GPU-2: depth_texture has no Inf values
      GPU-3: All depth values are in [0.0, 1.0] (normalized depth) or [near, far] (linear depth)
      GPU-4: No GPU validation errors reported by WebGPU error scope
```

---

## Edit Variant: Mid-Scene Edits

### Setup and Ingest

```
T-FP-2: Random scene with mid-scene edits

  Scene generation and ingest: same as T-FP-1
  Render frames 1-5: same as T-FP-1 (verify all invariants each frame)
```

### Edit Phase (After Frame 5)

```
  After frame 5, before frame 6:
    Select 10 random resident slots: E1..E10
    For each Ei:
      Select random voxel position (x, y, z) ∈ [0, 63]³
      Toggle the voxel (set if clear, clear if set)
      Record: old_version = chunk_version[Ei] before edit

  After all 10 edit kernels dispatch:
    For each Ei:
      EDIT-1: chunk_version[Ei] == old_version + 1
      EDIT-2: dirty_chunks bit for Ei is set
      EDIT-3: dirty_subregions[Ei] has correct bricklet bit set
      EDIT-4: If voxel was on a chunk boundary: boundary_touch_mask[Ei] has correct face bits
```

### Propagation Verification

```
  After propagation pass:
    For each Ei:
      PROP-1: stale_mesh[Ei] == 1
      PROP-2: stale_summary[Ei] == 1
    For each Ei with boundary edits:
      PROP-3: stale_mesh[adjacent_slot] == 1 for each touched face
    For all other resident slots (not in E1..E10 and not neighbors of boundary edits):
      PROP-4: stale_mesh[S] == 0  (no spurious stale flags)
      PROP-5: stale_summary[S] == 0
```

### Compaction Verification

```
  After compaction pass:
    COMP-1: queue_counts[0] (mesh) >= 10  (at least the 10 edited slots; more if boundary neighbors)
    COMP-2: queue_counts[1] (summary) >= 10
    COMP-3: mesh_rebuild_queue contains all of E1..E10
    COMP-4: summary_rebuild_queue contains all of E1..E10
    COMP-5: No duplicate entries in either queue (QUE-4)
```

### Rebuild and Render Verification (Frames 6-10)

```
  Render frames 6-10 with camera continuing to orbit:
    Frame 6: rebuild passes consume queues (budgeted)
      After R-1: for each rebuilt slot, mesh_version == chunk_version, stale_mesh cleared
      After I-3: for each rebuilt slot, summary_version == chunk_version, stale_summary cleared
      Mesh rebuild pass sets stale_meshlet for each rebuilt slot

    Frames 6-10: verify ALL per-frame invariants from the static test (OCC, FLG, SUM, AABB, PAL, IDX, VER, MAT, DRW, GPU)

    By frame 10 (assuming sufficient rebuild budget):
      ALL-1: stale_mesh == all zeros for resident slots
      ALL-2: stale_summary == all zeros for resident slots
      ALL-3: For all resident slots: mesh_version == chunk_version
      ALL-4: For all resident slots: summary_version == chunk_version
      ALL-5: Edited voxels are reflected in occupancy readback
      ALL-6: Meshes for edited chunks contain updated geometry
```

---

## Stress Variant: High Edit Rate

```
T-FP-3: Sustained edits every frame

  Scene: 200 chunks, random occupancy
  Ingest all, verify invariants

  For frames 1-20:
    Before each frame: edit 5 random voxels in random resident chunks
    Run full pipeline: edit → propagation → compaction → rebuild (budgeted) → render
    After each frame: verify all invariants (OCC through GPU)

  Key assertions:
    STRESS-1: No invariant violation across any of the 20 frames
    STRESS-2: Rebuild queues never exceed MAX_SLOTS (QUE-3)
    STRESS-3: If rebuild budget < stale count, stale bits persist correctly to next frame
    STRESS-4: Stale bits from frame N that were not rebuilt are re-compacted in frame N+1
    STRESS-5: No slot has stale_mesh == 0 with mesh_version != chunk_version
      (this would indicate a stale flag was cleared without a successful rebuild)
    STRESS-6: Total GPU validation errors across all 20 frames == 0
```

---

## Eviction Variant: Pool Pressure During Rendering

```
T-FP-4: Pool churn under render load

  Scene: 500 chunks (exceeds 1024-slot pool — requires streaming)
  Ingest first 300 chunks, fill pool partially

  For frames 1-10:
    Evict 20 LRU slots, ingest 20 new chunks from the remaining 200
    Render frame
    After each frame:
      CHURN-1: All resident slots satisfy all invariants
      CHURN-2: Evicted slots have chunk_resident_flags == 0 and chunk_version == 0
      CHURN-3: Newly uploaded slots have correct occupancy, coord, and version
      CHURN-4: No draw call references an evicted slot
      CHURN-5: No stale flag for an evicted slot causes a rebuild pass to process it
      CHURN-6: depth_texture has no NaN or Inf
```

---

## Consistency Properties (Must Hold After Every Frame in Every Variant)

```
P-FP-1: For every resident slot S:
  chunk_flags.is_empty == (popcount(occupancy_atlas[S]) == 0)

P-FP-2: For every resident slot S with is_empty == 0 and stale_summary == 0:
  chunk_aabb.min < chunk_aabb.max (component-wise)
  Every occupied voxel is inside the AABB

P-FP-3: For every resident slot S with stale_summary == 0:
  occupancy_summary bit b == 1 ⟺ bricklet b has at least one occupied voxel

P-FP-4: For every resident slot S:
  mesh_version[S] <= chunk_version[S]
  summary_version[S] <= chunk_version[S]

P-FP-5: For every non-resident slot S:
  No render pass, rebuild pass, or compaction pass reads derived data from S

P-FP-6: queue_counts[Q] == popcount(stale_Q bitset) after every compaction pass

P-FP-7: No duplicate entries exist within any single queue after compaction

P-FP-8: depth_texture contains only finite values in [0.0, 1.0] after every frame

P-FP-9: indirect_draw_buf entries reference only resident, non-empty slots with current meshes
```

These properties are the superset of all per-subsystem invariants. A single violation in any frame of any variant constitutes a test failure. The randomized nature of the inputs ensures coverage of edge cases that handcrafted tests cannot anticipate.
