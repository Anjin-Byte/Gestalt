# Meshlet Surface Clusters

**Type:** spec
**Status:** current
**Date:** 2026-03-21

> Sub-chunk surface cluster tier for finer-grained GPU visibility selection.

Extends the R‑4 occlusion cull pass with a two-phase dispatch; no other stage is affected.

Related: [pipeline-stages](pipeline-stages.md) (R‑4 two-tier cull), [gpu-chunk-pool](gpu-chunk-pool.md) (pool design pattern), [edit-protocol](edit-protocol.md) (dirty/stale/compaction machinery), [layer-model](layer-model.md) (Product 3 framing).

---

## What Meshlets Are and What They Buy

A meshlet is a small contiguous batch of triangles from a chunk surface mesh, assigned a
conservative world-space AABB. The GPU tests each meshlet's AABB against the Hi-Z pyramid
and emits indirect draw arguments only for passing meshlets.

At chunk granularity (current R‑4), a partially occluded chunk submits all its triangles.
With meshlets, only the unoccluded portion of the chunk submits triangles. That is the
primary payoff: **cost ∝ visible surface, not resident surface**.

Secondary benefit: meshlet bounds are tight enough that a normal-distribution cone can be
attached for conservative backface rejection before Hi-Z sampling.

---

## Product Layer

Meshlets are **Product 3 (camera-visibility structure)** exclusively.

- Derived from the chunk surface mesh (Product 2 artifact).
- Never consulted by Product 1 (ray traversal). Ray traversal reads `chunk_occupancy_atlas`
  and `occupancy_summary` only; it does not know meshlets exist.
- A "BVH over meshlets" (if ever added for raster cluster hierarchies) is a Product 3
  structure for raster selection. It is not a "BVH over voxels" and does not conflict with
  [traversal-acceleration](traversal-acceleration.md)'s explicit rejection of BVH for voxel ray traversal.

**Results from meshlet culling must never feed into Product 1 queries.** A meshlet rejected
by Hi-Z for the current camera position is still a valid target for a probe ray, a shadow
query, or a radiance cascade interval.

---

## Meshlet Descriptor

```
struct MeshletDesc {
    aabb_min:             vec3f,   // world-space conservative AABB minimum
    aabb_max:             vec3f,   // world-space conservative AABB maximum
    index_offset:         u32,     // offset into meshlet_index_pool (in u32 units)
    index_count:          u32,     // number of indices (triangles × 3)
    vertex_base:          u32,     // base into vertex_pool matching parent chunk's range
    chunk_slot:           u32,     // parent chunk slot (for version check and flag read)
    built_from_version:   u32,     // chunk_version[slot] at time of meshlet build
    _pad:                 u32,     // reserved; may hold normal cone encoding later
}
```

AABB must be **conservative** — never under-approximate. A false positive (culled geometry
that is actually visible) produces corruption. A false positive (visible geometry that is
drawn unnecessarily) is just wasted work. For voxel meshes, conservative AABBs are trivial
to compute: geometry is integer-grid-aligned, so the enclosing box in world space is exact.
The precision risk is in clip-space projection; err toward expanding the screen-space rect
slightly when uncertain.

---

## Pool Layout

```
meshlet_desc_pool    array<MeshletDesc>          // flat pool, all chunks
meshlet_index_pool   array<u32>                  // per-meshlet index data, all chunks
meshlet_range_table  array<MeshletRange, N_SLOTS> // per slot: where this chunk's meshlets live
```

```
struct MeshletRange {
    start: u32,   // first index into meshlet_desc_pool for this slot
    count: u32,   // number of meshlets for this slot (0 = no meshlets / not built)
}
```

`meshlet_range_table` is the same slot-indexed flat-array pattern used by every other
derived buffer in the pool. R‑4 phase 2 reads it identically to how R‑4 phase 1 reads
`chunk_aabb`.

`meshlet_index_pool` entries index into `vertex_pool` using the same base as the parent
chunk mesh. There is no second copy of vertex data.

---

## Generation Options

### Option S — Subchunk-Aligned (start here)

Partition the 64³ chunk into an 8×8×8 grid of subchunks. Each 8³ subchunk produces at most
one meshlet batch covering triangles whose origin voxels fall within that subregion. The
AABB is the integer-aligned 8³ world-space box of the subchunk.

**Pros:**
- Rebuild scope maps directly to `dirty_subregions` — only subchunks touched by an edit
  need re-meshing and re-clustering. Meshlet rebuild cost under edits is bounded and local.
- Bounds are exact by construction; no tightening step needed.
- Stable: equal edits produce equal meshlet layouts, no allocator churn.
- Meshlets per chunk are at most 512 (8³); empty subchunks have zero triangles and are
  trivially skipped by the bounds test.

**Cons:**
- Loose bounds for sparse subchunks (a few surface triangles still claim the full 8³ box).
- Meshlet count scales with chunk density, not surface density.

### Option A — Adaptive from Greedy Mesh Output (defer)

Run a meshlet builder on the full chunk greedy mesh using quality heuristics (vertex-
complete first, then primitive fill). Each meshlet is a batch of at most 64 vertices /
128 triangles.

**Pros:** tighter bounds where geometry clusters naturally; lower total meshlet count for
sparse chunks.

**Cons:** rebuild is full-chunk (cannot scope to subregion without re-running the full
builder); more complex rebuild logic; meshlet builder must run CPU-side or as GPU compute.

**Decision:** implement Option S first. Migrate to Option A if profiling shows that the
loose bounds of fixed 8³ subchunks are causing measurable false positives in Hi-Z tests.

---

## Rebuild Invariants

Meshlets are derived from the chunk surface mesh, which is derived from chunk occupancy.
The version chain is:

```
chunk_version[slot]           authoritative — bumped on every voxel edit
  → mesh rebuild              writes vertex/index pool; stamps mesh_version[slot]
    → meshlet rebuild         writes meshlet_desc/index pool; stamps meshlet_version[slot]
                              built_from_version = chunk_version[slot] at dispatch time
```

**Invariant 1 — Validity check:**
A meshlet is valid iff `MeshletDesc.built_from_version == chunk_version[slot]`.

**Invariant 2 — R‑4 fallback:**
R‑4 phase 2 must check `meshlet_version[slot] == chunk_version[slot]` before iterating
meshlets. If they differ (rebuild in flight), skip meshlet culling for this slot and emit
one chunk-level draw arg from `draw_metadata[slot]` instead. Do not drop the chunk.

**Invariant 3 — No incremental in-place writes:**
A meshlet rebuild always writes to a new region in `meshlet_desc_pool` and
`meshlet_index_pool`. The swap pass then updates `meshlet_range_table[slot]` atomically
and returns the old region to the freelist. Never patch meshlet data in place while R‑4
might be reading it.

**Invariant 4 — Co-rebuilding with mesh:**
Meshlet rebuild is triggered by a mesh rebuild completing, not independently. When the
mesh rebuild pass writes a new mesh for a slot, it also sets `stale_meshlet[slot]` in the
control plane. The compaction pass then enqueues the slot for meshlet rebuild on the next
eligible frame.

---

## Control Plane Additions

Following the edit-protocol pattern exactly: stale bitsets → compaction → work queues.
CPU must not write queues directly.

### New Control Plane Buffers

| Buffer | Owner | Written by | Read by |
|---|---|---|---|
| `stale_meshlet` | Edit protocol | Mesh rebuild pass (after mesh commit) | Compaction pass |
| `meshlet_rebuild_queue` | Edit protocol | Compaction pass | Meshlet build pass |
| `meshlet_version` | Chunk pool (control) | Meshlet build pass | R‑4 phase 2, swap pass |
| `ready_to_swap_meshlet` | Chunk pool (control) | Meshlet build pass | Swap pass |

### Flow

```
1. Mesh rebuild pass commits new mesh for slot
   → atomicOr stale_meshlet[slot / 32], (1u << (slot % 32))

2. Compaction pass scans stale_meshlet
   → appends slot to meshlet_rebuild_queue
   → clears stale_meshlet bit

3. Meshlet build pass (budgeted, consumes meshlet_rebuild_queue[0..N])
   → allocates region in meshlet_desc_pool + meshlet_index_pool
   → writes MeshletDesc[] for each meshlet in chunk
   → stamps built_from_version = current chunk_version[slot]
   → stamps meshlet_version[slot] = built_from_version
   → sets ready_to_swap_meshlet[slot]

4. Swap pass
   → for each slot where ready_to_swap_meshlet is set:
       verify built_from_version == chunk_version[slot]
         if match:   update meshlet_range_table[slot], free old region,
                     clear ready_to_swap_meshlet[slot]
         if mismatch: discard output, free new region,
                      clear ready_to_swap_meshlet[slot], re-queue
```

---

## Updated R‑4: Two-Phase Dispatch

R‑4 is extended to two phases. Each phase is a separate compute dispatch.

### Phase 1 — Chunk Coarse Cull (unchanged from current spec)

One thread per chunk slot.

```
for slot in 0..N_SLOTS:
  if !chunk_resident_flags[slot].is_resident: skip
  if chunk_flags[slot].is_empty:              skip
  if frustum_cull(chunk_aabb[slot]):          skip
  if hiz_cull(chunk_aabb[slot]):              skip
  append slot → chunk_visible_list
```

Output: `chunk_visible_list` (append buffer), `chunk_visible_count` (atomic u32).

### Phase 2 — Meshlet Fine Cull (new)

Indirect dispatch sized by `chunk_visible_count`. One workgroup per surviving chunk slot.

```
slot = chunk_visible_list[workgroup_id]

if meshlet_version[slot] != chunk_version[slot]:
  // Meshlet rebuild in flight — fall back to chunk-level draw
  emit DrawIndexedIndirectArgs from draw_metadata[slot]
  return

range = meshlet_range_table[slot]
for m in range.start .. range.start + range.count:
  desc = meshlet_desc_pool[m]
  if frustum_cull(desc.aabb_min, desc.aabb_max):      skip
  // optional: if backface_cone_cull(desc.normal_cone): skip
  if hiz_cull(desc.aabb_min, desc.aabb_max):           skip
  emit DrawIndexedIndirectArgs {
    index_count:    desc.index_count,
    instance_count: 1,
    first_index:    desc.index_offset,
    base_vertex:    desc.vertex_base,
    first_instance: 0,
  }
```

Output: `indirect_draw_buf` (same buffer as before), `visible_meshlet_count` (atomic u32).

R‑5 color pass is unchanged — it consumes `indirect_draw_buf` as before.

---

## Buffer Ownership

### New Data Plane Buffers

| Buffer | Owner | Written by | Read by | Lifetime |
|---|---|---|---|---|
| `meshlet_desc_pool` | Meshlet pool | Meshlet build pass | R‑4 phase 2 | Scene |
| `meshlet_index_pool` | Meshlet pool | Meshlet build pass | R‑5 indirect draw | Scene |
| `meshlet_range_table` | Meshlet pool | Swap pass | R‑4 phase 2 | Scene |

### New Control Plane Buffers

| Buffer | Owner | Written by | Read by | Lifetime |
|---|---|---|---|---|
| `stale_meshlet` | Edit protocol | Mesh rebuild pass | Compaction pass | Scene |
| `meshlet_rebuild_queue` | Edit protocol | Compaction pass | Meshlet build pass | Per-frame (consumed) |
| `meshlet_version` | Chunk pool | Meshlet build pass | R‑4 phase 2, swap pass | Scene |
| `ready_to_swap_meshlet` | Chunk pool | Meshlet build pass | Swap pass | Scene |

### New Per-Frame Buffers

| Buffer | Owner | Written by | Read by | Lifetime |
|---|---|---|---|---|
| `chunk_visible_list` | Cull pass | R‑4 phase 1 | R‑4 phase 2 | Per frame |
| `chunk_visible_count` | Cull pass | R‑4 phase 1 | R‑4 phase 2 dispatch args | Per frame |

---

## Memory Budget

For 1024 slots, Option S:

| Resource | Per slot | 1024 slots |
|---|---|---|
| `meshlet_desc_pool` (≤ 512 meshlets × 32B) | 16KB | 16MB |
| `meshlet_index_pool` (typical 30% surface density, 128 tri/meshlet) | ~50KB | ~50MB |
| `meshlet_range_table` | 8B | 8KB |
| `meshlet_version` + `stale_meshlet` + control | — | ~1MB total |
| **Total addition** | | **~67MB** |

In practice, most slots are empty or sparse. Realistic budget is closer to 10–20MB for a
typical test scene. The meshlet pool should use a freelist allocator matching the mesh pool
pattern (Option A fixed-reservation, Option B variable freelist — see [gpu-chunk-pool](gpu-chunk-pool.md)).

---

## What's Deferred

| Technique | Why |
|---|---|
| Normal cone backface culling | Useful when geometry is "closed-ish"; add only if profiling shows Hi-Z tests dominating cull pass time |
| Option A adaptive meshlets | Implement after Option S is validated and bounds looseness is measured |
| Meshlet LOD hierarchy | Out of scope; relevant only for Nanite-style multi-level cluster hierarchies |
| Mesh shader path | Compatible with this layout; indirect draw works without mesh shaders — defer until mesh shaders are reliably available across target browsers |

---

## What Needs to Be Built

| Component | Status | Blocks |
|---|---|---|
| Meshlet builder (Option S, CPU, per chunk) | Not implemented | Everything |
| `meshlet_desc_pool` + `meshlet_index_pool` allocation | Not implemented | Meshlet build |
| `meshlet_range_table` GPU buffer | Not implemented | R‑4 phase 2 |
| `stale_meshlet` + `meshlet_rebuild_queue` | Not implemented | Compaction integration |
| `meshlet_version` + `ready_to_swap_meshlet` | Not implemented | Swap pass |
| R‑4 phase 1 → `chunk_visible_list` output | Not implemented | R‑4 phase 2 |
| R‑4 phase 2 (meshlet cull + indirect write) | Not implemented | Reduced overdraw |

Build order: meshlet builder (Option S) → pool allocation → `meshlet_range_table` → swap
pass integration → R‑4 phase 1 `chunk_visible_list` output → R‑4 phase 2.

---

## See Also

- [pipeline-stages](pipeline-stages.md) — R‑4 stage definition; buffer ownership summary
- [gpu-chunk-pool](gpu-chunk-pool.md) — pool allocator pattern; slot lifecycle; mesh pool design
- [edit-protocol](edit-protocol.md) — `stale_meshlet` extends the dirty/compaction/version-swap pattern
- [layer-model](layer-model.md) — Product 3 definition; why meshlets must not filter Product 1 queries
- [traversal-acceleration](traversal-acceleration.md) — Product 1; "BVH over meshlets" is a different domain
