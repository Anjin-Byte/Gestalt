# Test: Edit Roundtrip (Edit → Dirty → Stale → Rebuild → Render)

**Type:** spec
**Status:** current
**Date:** 2026-03-22

> Proves the full edit pipeline is logically consistent: a voxel edit propagates through dirty detection, staleness expansion, work queue compaction, rebuild passes, and final rendering with correct visual output.

---

## What This Tests

The edit roundtrip transforms a single voxel write into updated GPU-resident derived products:

```
Voxel edit → dirty_chunks bit → stale_mesh + stale_summary → rebuild queues → R-1 mesh rebuild + I-3 summary rebuild → correct render
```

Each transition has a contract. This document defines the tests that prove those contracts hold across the full chain, including boundary propagation, multi-edit coalescing, and empty-to-occupied chunk transitions.

---

## Link 1: Edit → Dirty Propagation

**Claim:** A single voxel write produces the correct dirty_chunks bit, dirty_subregions bits, boundary_touch_mask, and version increment.

### Tests

```
T-E1-1: Single voxel dirty bit
  Write one voxel at (10, 20, 30) in slot S
  Assert: dirty_chunks[S >> 5] has bit (S & 31) set
  Assert: dirty_subregions[S * 16 + w] has the correct bricklet bit set
    (bricklet = (10/8, 20/8, 30/8) = (1, 2, 3), linear index = 1*64 + 2*8 + 3 = 83)
  Assert: chunk_version[S] incremented by 1

T-E1-2: Interior voxel does not set boundary mask
  Write one voxel at (32, 32, 32) in slot S (interior, not near any face)
  Assert: boundary_touch_mask[S] == 0x00

T-E1-3: Boundary voxel sets correct face bits
  Write one voxel at (0, 32, 32) in slot S (on -X face)
  Assert: boundary_touch_mask[S] & 0x01 != 0  (bit 0 = -X)
  Write one voxel at (63, 32, 32) in slot S (on +X face)
  Assert: boundary_touch_mask[S] & 0x02 != 0  (bit 1 = +X)

T-E1-4: Corner voxel sets three face bits
  Write one voxel at (0, 0, 0) in slot S
  Assert: boundary_touch_mask[S] & 0x15 == 0x15  (bits 0, 2, 4 = -X, -Y, -Z)

T-E1-5: Version increment is atomic and post-write
  Record v0 = chunk_version[S]
  Write occupancy for slot S
  Assert: chunk_version[S] == v0 + 1
  Assert: occupancy data is committed before version reads as incremented
```

---

## Link 2: Dirty → Stale Flags

**Claim:** The propagation pass correctly expands dirty_chunks into stale_mesh, stale_summary, and (for boundary edits) neighbor stale_mesh.

### Tests

```
T-E2-1: Dirty chunk sets own stale flags
  Set dirty_chunks bit for slot S
  Run propagation pass
  Assert: stale_mesh[S] == 1
  Assert: stale_summary[S] == 1

T-E2-2: Boundary edit propagates to neighbor
  Set dirty_chunks bit for slot S
  Set boundary_touch_mask[S] = 0x02  (+X face touched)
  Let A = adjacent slot in +X direction from S
  Run propagation pass
  Assert: stale_mesh[S] == 1
  Assert: stale_mesh[A] == 1
  Assert: dirty_chunks bit for A is now set (neighbor injection)

T-E2-3: Interior edit does not propagate to neighbors
  Set dirty_chunks bit for slot S
  Set boundary_touch_mask[S] = 0x00  (no faces touched)
  Let A = any adjacent slot of S
  Run propagation pass
  Assert: stale_mesh[A] == 0  (unless A was independently dirty)

T-E2-4: Propagation does not set stale_meshlet
  Set dirty_chunks bit for slot S
  Run propagation pass
  Assert: stale_meshlet[S] == 0
  (stale_meshlet is set by mesh rebuild pass, not propagation — STL-5)

T-E2-5: Multiple dirty chunks propagate independently
  Set dirty_chunks bits for slots S1, S2, S3 (non-adjacent)
  Run propagation pass
  Assert: stale_mesh and stale_summary set for all three
  Assert: no spurious stale flags on other slots
```

---

## Link 3: Stale Flags → Rebuild Queues

**Claim:** The compaction pass produces correct, deduplicated queue entries from stale bitsets.

### Tests

```
T-E3-1: Stale mesh produces mesh queue entry
  Set stale_mesh bit for slot S
  Run compaction pass
  Assert: mesh_rebuild_queue contains S
  Assert: queue_counts[0] >= 1

T-E3-2: Stale summary produces summary queue entry
  Set stale_summary bit for slot S
  Run compaction pass
  Assert: summary_rebuild_queue contains S
  Assert: queue_counts[1] >= 1

T-E3-3: Both stale flags produce entries in both queues
  Set stale_mesh and stale_summary for slot S
  Run compaction pass
  Assert: S appears in mesh_rebuild_queue
  Assert: S appears in summary_rebuild_queue

T-E3-4: No duplicates after boundary expansion
  Edit a boundary voxel in slot S, propagation marks S and neighbor A stale
  Run compaction pass
  Assert: mesh_rebuild_queue contains S exactly once
  Assert: mesh_rebuild_queue contains A exactly once
  Assert: queue_counts[0] == 2
  (QUE-4: no duplicate slot indices within a single compaction pass)

T-E3-5: Queue counts match stale popcount
  Set stale_mesh bits for K random slots
  Run compaction pass
  Assert: queue_counts[0] == K
```

---

## Link 4: Rebuild → Updated Derived Products

**Claim:** R-1 (mesh rebuild) and I-3 (summary rebuild) produce correct artifacts and stamp matching version tags.

### Tests

```
T-E4-1: Mesh rebuild produces new geometry
  Edit: set voxel at (17, 42, 31) in previously empty slot S
  Run full pipeline: propagation → compaction → R-1 mesh rebuild
  Assert: vertex/index pool for slot S contains non-zero geometry
  Assert: mesh_version[S] == chunk_version[S]
  Assert: stale_mesh[S] == 0  (cleared by rebuild)

T-E4-2: Summary rebuild updates flags
  Edit: set voxel at (17, 42, 31) in previously empty slot S
  Run full pipeline: propagation → compaction → I-3 summary rebuild
  Assert: chunk_flags.is_empty == 0
  Assert: occupancy_summary has correct bricklet bit set (bricklet (2, 5, 3))
  Assert: chunk_aabb encloses (17, 42, 31)
  Assert: summary_version[S] == chunk_version[S]
  Assert: stale_summary[S] == 0  (cleared by rebuild)

T-E4-3: Version mismatch triggers re-queue
  Edit slot S (version becomes V1)
  Start R-1 rebuild for slot S
  While rebuild is in flight, edit slot S again (version becomes V2)
  Rebuild completes with built_from_version == V1
  Swap pass: V1 != V2 → discard, re-queue
  Assert: stale_mesh[S] == 1 (re-queued)
  Assert: mesh_version[S] != chunk_version[S] (artifact not promoted)

T-E4-4: Mesh rebuild sets stale_meshlet
  Run R-1 mesh rebuild for slot S
  Assert: stale_meshlet[S] == 1
  (Meshlet staleness is a consequence of mesh staleness, not of voxel edits)
```

---

## Link 5: Render Produces Correct Visual

**Claim:** After the full edit roundtrip, rendering reflects the edit.

### Tests

```
T-E5-1: New voxel appears in render
  Start with empty chunk at slot S
  Edit: set voxel at (32, 32, 32)
  Run full pipeline through swap pass
  Render one frame
  Assert: indirect_draw_buf entry for slot S has non-zero vertex count
  Assert: depth_texture has a finite depth value at the projected screen position of voxel (32, 32, 32)

T-E5-2: Removed voxel disappears from render
  Start with a single-voxel chunk at slot S, voxel at (32, 32, 32)
  Edit: clear voxel at (32, 32, 32)
  Run full pipeline through swap pass
  Assert: chunk_flags.is_empty == 1
  Assert: indirect_draw_buf entry for slot S has zero vertex count (or slot is culled)
```

---

## Cross-Cutting Scenarios

### Boundary Edit → Neighbor Dirty Propagation

```
T-X1: Boundary edit roundtrip
  Setup: two adjacent chunks, slot S (chunk at (0,0,0)) and slot A (chunk at (1,0,0))
  Both chunks have occupancy; meshes are current (mesh_version == chunk_version)
  Edit: set voxel at (63, 32, 32) in slot S  (on +X boundary)

  After edit kernel:
    Assert: dirty_chunks[S] == 1
    Assert: boundary_touch_mask[S] & 0x02 != 0  (+X face)
    Assert: chunk_version[S] incremented

  After propagation:
    Assert: stale_mesh[S] == 1
    Assert: stale_mesh[A] == 1  (neighbor propagation)
    Assert: stale_summary[S] == 1

  After compaction:
    Assert: mesh_rebuild_queue contains both S and A
    Assert: summary_rebuild_queue contains S

  After rebuild:
    Assert: mesh_version[S] == chunk_version[S]
    Assert: mesh_version[A] == chunk_version[A]
    (A's mesh is rebuilt because face visibility depends on neighbor occupancy)
```

### Multiple Edits Coalesce in One Frame

```
T-X2: Multi-edit coalescing
  Edit 50 voxels across 3 chunks (slots S1, S2, S3) in a single frame
  Each edit kernel atomicOr's dirty bits — all 50 edits land before propagation

  After all edit dispatches:
    Assert: dirty_chunks has exactly bits S1, S2, S3 set
    Assert: chunk_version[S1] incremented by the number of edits in S1
    Assert: dirty_subregions[S1] has union of all affected bricklets

  After propagation:
    Assert: stale flags set for S1, S2, S3 (and any boundary neighbors)
    Assert: no stale flags for untouched chunks

  After compaction + rebuild:
    Assert: each slot rebuilt once (not once per voxel edit)
    Assert: final mesh reflects all 50 edits, not a subset
```

### Edit to Empty Chunk → Chunk Becomes Non-Empty

```
T-X3: Empty-to-occupied transition
  Setup: slot S is resident, is_empty == 1, occupancy all zero, mesh is empty
  Edit: set voxel at (10, 10, 10)

  After full pipeline:
    Assert: chunk_flags.is_empty == 0  (cleared by I-3 summary rebuild)
    Assert: occupancy_summary has bricklet (1, 1, 1) bit set
    Assert: chunk_aabb is non-degenerate (min < max component-wise)
    Assert: mesh has non-zero geometry
    Assert: indirect_draw_buf entry for slot S has non-zero vertex count
```

---

## Consistency Properties (Hold After Any Edit Roundtrip)

```
P-E1: For every slot S with chunk_version[S] > 0 and is_resident == 1:
  If stale_mesh[S] == 0:
    mesh_version[S] == chunk_version[S]

P-E2: For every slot S with chunk_version[S] > 0 and is_resident == 1:
  If stale_summary[S] == 0:
    summary_version[S] == chunk_version[S]

P-E3: After propagation completes:
  dirty_chunks is a subset of (stale_mesh ∪ stale_summary)
  (every dirty chunk has at least stale_mesh and stale_summary set)

P-E4: After compaction completes:
  queue_counts[0] == popcount(stale_mesh)
  queue_counts[1] == popcount(stale_summary)

P-E5: After all rebuilds complete and swap pass succeeds:
  stale_mesh == all zeros (if budget was sufficient)
  stale_summary == all zeros (if budget was sufficient)
  For all resident slots: mesh_version == chunk_version and summary_version == chunk_version
```

These properties are testable as assertions after any edit roundtrip and serve as the bridge between per-link tests and full-pipeline invariant checking.
