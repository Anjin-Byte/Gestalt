# Test: Pool Lifecycle (Allocate → Upload → Render → Edit → Rebuild → Evict)

**Type:** spec
**Status:** current
**Date:** 2026-03-22

> Proves the full slot lifecycle is correct: every state transition in a chunk slot's lifetime maintains data integrity, version consistency, and clean reuse after eviction.

---

## What This Tests

A chunk slot passes through a complete lifecycle:

```
Allocate → Upload (I-2) → Render (R-2..R-5) → Edit → Rebuild → Evict → Reallocate
```

Each transition changes authoritative and derived state. This document defines the tests that prove those transitions are correct, that pool exhaustion is handled without corruption, and that slot reuse after eviction starts clean.

---

## Phase 1: Allocation

**Claim:** A freshly allocated slot is in a known initial state with no residual data from previous occupants.

### Tests

```
T-P1-1: Fresh slot version is zero
  Allocate slot S (first time, never used)
  Assert: chunk_version[S] == 0

T-P1-2: Fresh slot is not resident
  Allocate slot S (before upload)
  Assert: chunk_resident_flags[S] == 0

T-P1-3: Fresh slot has no stale flags
  Allocate slot S
  Assert: stale_mesh[S] == 0
  Assert: stale_summary[S] == 0
  Assert: stale_lighting[S] == 0
  Assert: stale_meshlet[S] == 0

T-P1-4: Fresh slot has no dirty bits
  Allocate slot S
  Assert: dirty_chunks bit for S == 0
  Assert: boundary_touch_mask[S] == 0x00

T-P1-5: Derived version tags are zero
  Allocate slot S
  Assert: mesh_version[S] == 0
  Assert: summary_version[S] == 0
  Assert: gi_cache_version[S] == 0
  Assert: meshlet_version[S] == 0
```

---

## Phase 2: Upload (I-2)

**Claim:** After upload, the slot is resident with valid authoritative data and incremented version.

### Tests

```
T-P2-1: Upload sets resident flag
  Allocate slot S, upload occupancy via I-2
  Assert: chunk_resident_flags[S] == 1

T-P2-2: Upload sets chunk coordinate
  Upload chunk at world coord (3, 7, 11) to slot S via I-2
  Assert: chunk_coord[S] == (3, 7, 11, 0)

T-P2-3: Upload increments version from zero
  Allocate slot S (version == 0)
  Upload via I-2
  Assert: chunk_version[S] > 0

T-P2-4: Occupancy data matches input
  Write known pattern (alternating 0xAA / 0x55 words) via I-2 to slot S
  Readback chunk_occupancy_atlas[S]
  Assert: every word matches the input pattern

T-P2-5: Upload to occupied slot increments version additively
  Upload to slot S (version becomes V1)
  Upload again to slot S with new data (re-upload / hot-reload)
  Assert: chunk_version[S] > V1
```

---

## Phase 3: Render (R-2..R-5)

**Claim:** A resident, non-stale slot produces valid render output.

### Tests

```
T-P3-1: Resident slot with current mesh renders
  Upload chunk to slot S via I-2
  Run I-3 (summary rebuild)
  Run R-1 (mesh rebuild) — mesh_version[S] == chunk_version[S]
  Render frame (R-2 through R-5)
  Assert: indirect_draw_buf entry for slot S references valid vertex/index ranges
  Assert: no GPU validation errors during render

T-P3-2: Non-resident slot is excluded from render
  Evict slot S (chunk_resident_flags[S] == 0)
  Render frame
  Assert: slot S does not appear in any draw call
  Assert: indirect_draw_buf entry for slot S has zero vertex count or is skipped by culling

T-P3-3: Stale mesh slot uses old mesh until rebuild
  Slot S is resident, mesh is current
  Edit slot S (stale_mesh[S] becomes 1, chunk_version increments)
  Render frame before rebuild pass runs
  Assert: render uses the existing (old) mesh — no crash, no undefined data
  Assert: mesh_version[S] < chunk_version[S] (stale, but still renderable)
```

---

## Phase 4: Edit → Version Increment

**Claim:** An edit to a resident slot correctly increments the version and sets dirty state without corrupting other slot data.

### Tests

```
T-P4-1: Edit increments version
  Slot S is resident with chunk_version[S] == V
  Perform voxel edit on slot S
  Assert: chunk_version[S] == V + 1

T-P4-2: Edit sets dirty bit
  Perform voxel edit on slot S
  Assert: dirty_chunks bit for S == 1

T-P4-3: Edit does not affect other slots
  Slots S and T are both resident
  Record: version_T = chunk_version[T]
  Perform voxel edit on slot S only
  Assert: chunk_version[T] == version_T  (unchanged)
  Assert: dirty_chunks bit for T == 0  (unchanged)

T-P4-4: Multiple edits to same slot accumulate version
  Perform 5 edits to slot S in one frame
  Assert: chunk_version[S] incremented by 5 (one atomicAdd per edit)
  Assert: dirty_chunks bit for S == 1 (idempotent — already set after first edit)
```

---

## Phase 5: Rebuild → Stale Cleared, Version Stamped

**Claim:** After rebuild, derived products are current and stale flags are cleared.

### Tests

```
T-P5-1: Mesh rebuild clears stale and stamps version
  Slot S has stale_mesh == 1, chunk_version == V
  Run R-1 mesh rebuild for slot S
  Swap pass succeeds (built_from_version == V == chunk_version)
  Assert: stale_mesh[S] == 0
  Assert: mesh_version[S] == V

T-P5-2: Summary rebuild clears stale and stamps version
  Slot S has stale_summary == 1, chunk_version == V
  Run I-3 summary rebuild for slot S
  Swap pass succeeds
  Assert: stale_summary[S] == 0
  Assert: summary_version[S] == V

T-P5-3: Mesh rebuild sets stale_meshlet
  Run R-1 mesh rebuild for slot S (commits new mesh)
  Assert: stale_meshlet[S] == 1
  (Meshlet staleness is a consequence of mesh change — STL-4)

T-P5-4: Rebuild during concurrent edit triggers re-queue
  Slot S has chunk_version == V, stale_mesh == 1
  R-1 begins rebuild, captures built_from_version = V
  During rebuild, another edit increments chunk_version to V+1
  Swap pass: V != V+1 → discard, re-queue
  Assert: stale_mesh[S] == 1  (still stale)
  Assert: mesh_version[S] != chunk_version[S]  (not updated)
```

---

## Phase 6: Eviction

**Claim:** Eviction clears the resident flag, resets the version, and makes all derived data undefined. No subsequent consumer reads stale data from a previous occupant.

### Tests

```
T-P6-1: Eviction clears resident flag
  Slot S is resident
  Evict slot S
  Assert: chunk_resident_flags[S] == 0

T-P6-2: Eviction resets version to zero
  Slot S has chunk_version[S] == 42
  Evict slot S
  Assert: chunk_version[S] == 0  (VER-2)

T-P6-3: Evicted slot excluded from all passes
  Evict slot S
  Assert: stale bits for slot S are undefined (STL-7) — no rebuild pass should process S
  Assert: slot S does not appear in render draw calls

T-P6-4: Derived version tags become meaningless after eviction
  Slot S had mesh_version == 42, summary_version == 42
  Evict slot S (chunk_version resets to 0)
  Assert: mesh_version[S] and summary_version[S] are stale artifacts
  (They may still read 42, but chunk_resident_flags == 0 gates all consumers)
```

---

## Cross-Cutting Scenarios

### Pool Exhaustion and LRU Eviction

```
T-X1: Allocate all slots → pool full → evict LRU → reallocate
  For i in 0..1024:
    Allocate slot, upload chunk via I-2, run I-3, run R-1
  Assert: all 1024 slots are resident

  Attempt to allocate slot 1025:
    Assert: pool is full, allocation requires eviction
    Evict LRU slot (slot E, the least recently used)
    Assert: chunk_resident_flags[E] == 0
    Assert: chunk_version[E] == 0

  Allocate new chunk into slot E:
    Upload via I-2
    Assert: chunk_resident_flags[E] == 1
    Assert: chunk_version[E] > 0
    Assert: chunk_coord[E] == new chunk's world coord (not the evicted chunk's coord)

  Run I-3 + R-1 for slot E:
    Assert: all derived products reflect the new chunk, not the evicted occupant
    Assert: no corruption in adjacent slots (E-1, E+1)
```

### Slot Reuse Cleanliness

```
T-X2: Slot reuse after eviction starts clean
  Allocate slot S, upload chunk with all-ones occupancy
  Run I-3: is_empty == 0, AABB covers full chunk
  Run R-1: mesh has geometry
  Evict slot S

  Reallocate slot S, upload chunk with all-zero occupancy
  Run I-3:
    Assert: is_empty == 1  (not carrying over old flags)
    Assert: occupancy_summary == all zeros
    Assert: AABB is degenerate
  Run R-1:
    Assert: mesh has zero geometry (not carrying over old mesh)
    Assert: mesh_version[S] == chunk_version[S]

T-X3: No ghost data after eviction and reuse
  Allocate slot S, upload chunk with specific voxel pattern P1
  Run full pipeline, render one frame — visual matches P1
  Evict slot S

  Reallocate slot S, upload chunk with different pattern P2
  Run full pipeline, render one frame
  Assert: visual matches P2, not P1
  Assert: no voxels from P1 appear in readback of chunk_occupancy_atlas[S]
```

### Rapid Allocate-Evict Cycling

```
T-X4: Rapid cycling does not leak state
  For i in 0..100:
    Allocate slot S with chunk i
    Upload via I-2
    Run I-3 + R-1
    Render one frame
    Evict slot S

  After final eviction:
    Assert: chunk_resident_flags[S] == 0
    Assert: chunk_version[S] == 0
    Assert: no dirty bits set for S
    Assert: no stale bits set for S (or if set, they are gated by non-resident flag)
```

---

## Consistency Properties (Hold at Any Point in Lifecycle)

```
P-L1: For every slot S:
  If chunk_resident_flags[S] == 0:
    No render pass, rebuild pass, or compaction pass may read derived data from S

P-L2: For every slot S with chunk_resident_flags[S] == 1:
  chunk_version[S] > 0
  chunk_coord[S] is a valid world coordinate

P-L3: For every slot S with chunk_resident_flags[S] == 1 and stale_mesh[S] == 0:
  mesh_version[S] == chunk_version[S]

P-L4: For every slot S with chunk_resident_flags[S] == 1 and stale_summary[S] == 0:
  summary_version[S] == chunk_version[S]

P-L5: Slot index S is either:
  (a) resident and valid (all authoritative data set, derived data current or stale-but-queued), or
  (b) not resident (derived data is undefined, no consumer may read it)
  There is no third state.

P-L6: After eviction of slot S and before reallocation:
  chunk_version[S] == 0
  chunk_resident_flags[S] == 0
```

These properties are testable as assertions after any lifecycle transition and serve as the invariant backbone for the pool manager implementation.
