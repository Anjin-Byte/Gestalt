# Test: Pool Invariants (VER / COORD / DRT)

**Type:** spec
**Status:** current
**Date:** 2026-03-22

> Proves the chunk pool state machine is valid: versions never regress, coordinates are unique and immutable, dirty bits track edits faithfully, and the full slot lifecycle is sound.

---

## What This Tests

The chunk pool manages a fixed set of slots through a lifecycle:

```
free → alloc → upload → render → edit → rebuild → ... → evict → free
```

Two buffers define pool identity and versioning:

```
chunk_coord    (authoritative, per-slot world position)
chunk_version  (authoritative, per-slot monotonic counter)
```

Dirty bits (stale_mesh, stale_summary, stale_lighting) in `chunk_flags` track which derived products need rebuilding. This document defines the tests that prove the pool state machine preserves all invariants across transitions.

---

## Structures Under Test

| Buffer | Invariants | Spec |
|---|---|---|
| `chunk_version` | VER-1 through VER-6 | `data/chunk-version.md` |
| `chunk_coord` | COORD-1 through COORD-4 | `data/chunk-coord.md` |
| `chunk_flags` (dirty bits) | DRT-1 through DRT-4 | `data/chunk-flags.md` (bits 4-6) |

Dirty-bit invariants (referenced as DRT-1 through DRT-4 in this document) map to the stale flag semantics defined in chunk-flags.md:

| ID | Invariant | Source |
|---|---|---|
| DRT-1 | `stale_mesh == 1` implies `chunk_version[slot] > mesh_version[slot]` | FLG-5 |
| DRT-2 | `stale_summary == 1` implies `chunk_version[slot] > summary_version[slot]` | Edit protocol |
| DRT-3 | After a rebuild pass clears a stale bit, the corresponding derived version tag equals `chunk_version[slot]` | VER-5 |
| DRT-4 | After an edit, all relevant stale bits are set before the next frame's dispatch | Edit protocol ordering |

---

## 1. No Double-Allocation of Slots (COORD-2)

**Claim:** No two resident slots ever share the same (x, y, z) chunk coordinate.

```
T-POOL-DEDUP-1: Duplicate rejection
  Allocate slot A at coord (5, 3, -2)
  Attempt to allocate slot B at coord (5, 3, -2)
  Assert: second allocation fails or returns the existing slot A

T-POOL-DEDUP-2: Adjacent coords are distinct
  Allocate slots at (0,0,0), (1,0,0), (0,1,0), (0,0,1)
  Assert: all four allocations succeed (different coords)
  Assert: each slot has a unique slot index

T-POOL-DEDUP-3: Reuse after eviction
  Allocate slot at coord (5, 3, -2)
  Evict that slot
  Allocate a new slot at coord (5, 3, -2)
  Assert: allocation succeeds (coord is free again)

T-POOL-DEDUP-4: Exhaustive uniqueness sweep
  Allocate N slots with random unique coords
  For every pair of resident slots (i, j) where i != j:
    Assert: chunk_coord[i].xyz != chunk_coord[j].xyz

T-POOL-DEDUP-5: Negative coordinate dedup
  Allocate at (-1, 0, 0) and (1, 0, 0)
  Assert: both succeed (sign matters)
  Attempt to re-allocate at (-1, 0, 0)
  Assert: fails (duplicate)
```

---

## 2. Version Never Decrements Within a Lifetime (VER-1)

**Claim:** `chunk_version[slot]` is monotonically non-decreasing from allocation to eviction.

```
T-VER-MONO-1: Sequential edits
  Allocate slot S
  Record v0 = chunk_version[S]
  Perform 100 sequential edits
  After each edit i, record v_i = chunk_version[S]
  Assert: v_0 < v_1 < v_2 < ... < v_100

T-VER-MONO-2: Upload increments
  Allocate slot S, record v0
  Upload occupancy data
  Assert: chunk_version[S] > v0

T-VER-MONO-3: Multiple uploads
  Upload to slot S three times with different data
  Record version after each upload: v1, v2, v3
  Assert: v1 < v2 < v3

T-VER-MONO-4: No decrement on rebuild
  Perform an edit (version increments)
  Run I-3 summary rebuild
  Assert: chunk_version[S] is unchanged (rebuilds don't modify chunk_version)
  Run R-1 mesh rebuild
  Assert: chunk_version[S] is unchanged
```

---

## 3. Version Reset on Eviction (VER-2)

**Claim:** After eviction, `chunk_version[slot]` is reset to 0, and re-allocation starts fresh.

```
T-VER-RESET-1: Eviction clears version
  Allocate slot S, perform 10 edits
  Assert: chunk_version[S] >= 10
  Evict slot S
  Assert: chunk_version[S] == 0

T-VER-RESET-2: Re-allocation starts at 0
  Allocate slot S, edit, evict
  Re-allocate the same slot index for a different coord
  Assert: chunk_version[S] == 0 (before any upload)
  Upload occupancy
  Assert: chunk_version[S] == 1
```

---

## 4. Coordinate Immutability (COORD-4)

**Claim:** `chunk_coord[slot]` does not change between allocation and eviction.

```
T-COORD-IMMUT-1: Coord survives edits
  Allocate slot S at coord (7, -3, 12)
  Perform 50 edits to occupancy/material data
  Assert: chunk_coord[S] == (7, -3, 12, 0)

T-COORD-IMMUT-2: Coord survives rebuilds
  Allocate slot, run I-3, run R-1
  Assert: chunk_coord[S] unchanged

T-COORD-IMMUT-3: W channel stays zero (COORD-1)
  Allocate 100 slots with random coords
  For each slot S:
    Assert: chunk_coord[S].w == 0
  Perform edits and rebuilds
  For each slot S:
    Assert: chunk_coord[S].w == 0

T-COORD-IMMUT-4: Write-after-alloc rejected
  Allocate slot S at coord (1, 2, 3)
  Attempt to overwrite chunk_coord[S] with (4, 5, 6) via pool API
  Assert: write is rejected or no-op
  Assert: chunk_coord[S] == (1, 2, 3, 0)
```

---

## 5. Eviction Clears All Derived Data

**Claim:** When a slot is evicted, all derived products and metadata are invalidated so no stale data can leak into a future allocation.

```
T-EVICT-CLEAR-1: Flags cleared
  Allocate slot S, upload data, run I-3
  Assert: chunk_flags[S].is_resident == 1
  Evict slot S
  Assert: chunk_flags[S].is_resident == 0

T-EVICT-CLEAR-2: Version reset
  Evict slot S
  Assert: chunk_version[S] == 0

T-EVICT-CLEAR-3: Derived version tags reset
  After eviction of slot S:
    Assert: mesh_version[S] == 0
    Assert: summary_version[S] == 0

T-EVICT-CLEAR-4: No consumer reads evicted slot
  Evict slot S
  Run I-3, R-1, R-4 (summary, mesh, cull)
  Assert: no pipeline stage reads from slot S
  (Verify via: slot S's occupancy/flags not accessed, or access returns
   is_resident == 0 causing early skip.)

T-EVICT-CLEAR-5: Occupancy undefined after eviction (OCC-5)
  Evict slot S
  Assert: reading occupancy_atlas[S] is not relied upon by any pipeline stage
  Re-allocate slot S with new data
  Assert: new data completely overwrites old data (no bleed-through)
```

---

## 6. Dirty Bit Tracking (DRT-1 through DRT-4)

**Claim:** Dirty bits faithfully track which derived products are stale.

```
T-DRT-EDIT-1: Edit sets stale bits
  Allocate slot S, upload, run I-3 and R-1 (all clean)
  Assert: stale_mesh == 0, stale_summary == 0
  Edit one voxel
  Assert: stale_mesh == 1
  Assert: stale_summary == 1

T-DRT-EDIT-2: Rebuild clears stale bit
  From T-DRT-EDIT-1 state (stale after edit):
  Run R-1 (mesh rebuild)
  Assert: stale_mesh == 0
  Assert: mesh_version[S] == chunk_version[S]
  Run I-3 (summary rebuild)
  Assert: stale_summary == 0
  Assert: summary_version[S] == chunk_version[S]

T-DRT-EDIT-3: Multiple edits before rebuild
  Perform 5 edits without rebuilding
  Assert: stale_mesh == 1 (stays set, doesn't accumulate)
  Run R-1
  Assert: stale_mesh == 0
  Assert: mesh_version[S] == chunk_version[S]

T-DRT-EDIT-4: Edit during rebuild
  Start R-1 mesh rebuild
  Concurrently edit a voxel (version increments)
  After R-1 completes:
    If mesh_version[S] < chunk_version[S]:
      Assert: stale_mesh == 1 (rebuild was based on old version)
    Else:
      Assert: stale_mesh == 0

T-DRT-EDIT-5: Stale lighting on material change
  Edit a voxel's material (not occupancy)
  Assert: stale_lighting == 1
  Assert: stale_summary == 1

T-DRT-FRESH-1: Fresh allocation has no stale bits
  Allocate slot S, upload occupancy, run I-3
  Assert: stale_mesh == 0 or stale_mesh == 1
    (stale_mesh may be 1 since mesh hasn't been built yet — depends on protocol)
  Assert: stale_summary == 0 (I-3 just ran)
```

---

## 7. Full Slot Lifecycle

**Claim:** The complete lifecycle alloc -> upload -> render -> edit -> rebuild -> evict is internally consistent at every transition.

```
T-LIFECYCLE-1: Happy path
  Step 1 — Alloc:
    Allocate slot S at coord (3, -1, 7)
    Assert: chunk_coord[S] == (3, -1, 7, 0)
    Assert: chunk_version[S] == 0
    Assert: is_resident == 1

  Step 2 — Upload:
    Upload known occupancy pattern
    Assert: chunk_version[S] == 1
    Assert: occupancy_atlas[S] matches uploaded data (readback)

  Step 3 — Summarize:
    Run I-3
    Assert: stale_summary == 0
    Assert: chunk_flags consistent with occupancy (FLG-1, FLG-2)
    Assert: occupancy_summary consistent with atlas (SUM-1)

  Step 4 — Mesh:
    Run R-1
    Assert: stale_mesh == 0
    Assert: mesh_version[S] == chunk_version[S]

  Step 5 — Render:
    Run R-2 through R-5 (depth, cull, traverse, shade)
    Assert: no errors, slot S is processed

  Step 6 — Edit:
    Edit voxel at (10, 20, 30): set occupied
    Assert: chunk_version[S] == 2
    Assert: stale_mesh == 1
    Assert: stale_summary == 1
    Assert: chunk_coord[S] unchanged

  Step 7 — Rebuild:
    Run I-3
    Assert: stale_summary == 0
    Assert: summary reflects the edit
    Run R-1
    Assert: stale_mesh == 0

  Step 8 — Evict:
    Evict slot S
    Assert: is_resident == 0
    Assert: chunk_version[S] == 0
    Assert: coord (3, -1, 7) is available for re-allocation

T-LIFECYCLE-2: Rapid alloc/evict cycling
  For 100 iterations:
    Allocate slot at random coord
    Upload minimal occupancy
    Evict slot
  Assert: no slot leaks (free count returns to initial value)
  Assert: no version accumulation across cycles (always resets to 0)

T-LIFECYCLE-3: Pool exhaustion and recovery
  Allocate MAX_SLOTS slots
  Attempt to allocate one more
  Assert: allocation fails (pool full)
  Evict one slot
  Attempt to allocate again
  Assert: allocation succeeds
```

---

## Swap Validation (VER-6)

**Claim:** When a slot is reused (evict + re-alloc), version-based staleness checks correctly distinguish old vs. new data.

```
T-SWAP-1: Stale reference detection
  Allocate slot S, edit to version 5
  Record mesh_version = 5 (mesh is current)
  Evict slot S (version resets to 0)
  Re-allocate slot S for a different chunk
  Upload (version becomes 1)
  Assert: mesh_version (still 0 after eviction reset) != chunk_version (1)
  Assert: stale_mesh == 1 (mesh must be rebuilt for new chunk)

T-SWAP-2: No version collision across lifetimes
  Allocate slot S, edit to version 3, evict
  Re-allocate slot S, edit to version 3
  These are different lifetimes — consumers must not confuse them
  Assert: any cached reference from the first lifetime is invalid
    (enforced by: eviction resets all derived version tags to 0)
```

---

## Consistency Properties (Hold for Any Valid Pool State)

```
P-POOL-1: For every pair of resident slots (i, j) where i != j:
  chunk_coord[i].xyz != chunk_coord[j].xyz

P-POOL-2: For every resident slot S:
  chunk_version[S] >= mesh_version[S]
  chunk_version[S] >= summary_version[S]

P-POOL-3: For every resident slot S:
  chunk_coord[S].w == 0

P-POOL-4: For every non-resident slot S:
  chunk_version[S] == 0

P-POOL-5: For every resident slot S:
  stale_mesh == (mesh_version[S] != chunk_version[S])
  stale_summary == (summary_version[S] != chunk_version[S])
```

These properties are testable as assertions after any pool operation and serve as preconditions for all pipeline stages that index by slot.
