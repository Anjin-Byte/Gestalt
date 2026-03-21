# Traversal Acceleration

Design for world-space ray traversal over the canonical chunk occupancy structure.
Consumed by A&W inner loops, GI probe rays, radiance cascade interval queries, picking, and shadow tests.

Related: [[layer-model]] (Product 1), [[chunk-field-registry]] (fields used), [[pipeline-stages]] (Stage R-6).

---

## Background: State of the Art in DDA

Classic Amanatides & Woo (1987) is still undefeated in spirit. The step from one voxel to the next costs two float comparisons and one addition. Every improvement since falls into one of four buckets:

**1. Make each step cheaper.**
A&W's "which axis wins?" branch maps poorly to GPU warps. Branchless axis-selection (PBRT, Xiao et al.) removes the divergence. Xiao et al. report 1.42×–2.67× speedup from branch-divergence reduction alone in GPU 3D-DDA.

**2. Traverse multiple scales.**
A&W over a uniform grid wastes steps on empty space. Two-level grids (Kalojanov et al.), sparse voxel octrees (Laine & Karras), and VDB hierarchies all address this by traversing coarse first, descending only where needed.

**3. Skip empty space more aggressively.**
Majorant grids, sparse grids, and octrees layer DDA over coarser structures so traversal emits useful segments instead of stepping through emptiness one cell at a time. PBRT's DDA majorant iterator is the modern canonical example.

**4. Improve coherence.**
On SIMT machines, rays going in different directions cause warp divergence. Packet tracing, ray sorting, and ray reordering recover utilization for coherent batches. Incoherent secondary/GI rays are harder and benefit less unless actively reordered.

The question for this engine is not "what is the state of the art in the abstract?" but: **which invariants of our world can be exploited so hard they squeal?**

---

## Gestalt's Traversal Invariants

These are fixed properties of the runtime world structure. Traversal should be designed around them rather than treating the world as a generic scene.

### Invariant 1 — Binary first, material second

`opaque_mask` answers the most important traversal question cheaply: *is this voxel blocking?*

Traversal is occupancy-first. Material data (`palette`, `index_buf`) is not touched until a hit is confirmed or highly likely. Material fetch is a second-stage query, never part of the hot path.

### Invariant 2 — Chunked, sparse, power-of-two

Chunks are fixed at 64³. This gives:
- Chunk/local coordinate mapping via shifts and masks (no division in non-negative domains)
- Predictable memory layout; no pointer chasing within a chunk
- Natural two-level DDA: chunk DDA → voxel DDA

### Invariant 3 — Hot occupancy is stored as u64 Y-columns

`opaque_mask[x * 64 + z]` is a u64 holding the entire Y-column at `(x, z)`.

This is the single most engine-specific optimization lever available. A plain DDA sees one voxel at a time. The column layout gives 64 Y voxels at once for the cost of one 64-bit load.

Consequences:
- Point occupancy is a single bit test: `(opaque_mask[x*64+z] >> y) & 1`
- Vertical runs of empty/occupied cells can be skipped with `ctz`/`clz`/`tzcnt` bit scans rather than individual voxel steps
- For rays with a meaningful Y component, column scans can skip empty runs far faster than per-step DDA
- Delay material lookup until bit test confirms hit

### Invariant 4 — Padding keeps queries local

The 1-voxel padding border exists to eliminate boundary checks in the hot loop. Neighbor occupancy at chunk edges is pre-populated from adjacent chunks during ingestion.

Traversal should be structured so:
- Neighbor and occupancy queries within a chunk never touch bounds checks
- Chunk-crossing is handled by the outer coarse DDA, not by branching inside the inner loop
- Chunk transitions are always a coarse-level event

### Invariant 5 — Empty space is most efficiently skipped at chunk granularity

Chunks are sparse and hash-mapped. The first filter should not be exotic — it should be:
- Is this chunk resident?
- Is `chunk_flags.is_empty` set?

Then skip absent or empty chunks entirely in the chunk-level DDA, before ever descending.

This gives most of the benefit of hierarchical traversal without a full octree.

### Invariant 6 — Two query modes with different termination conditions

| Mode | Used by | Termination |
|---|---|---|
| **First-hit** | Camera visibility, picking, collision, line-of-sight | Stop at first occupied voxel |
| **Segment stream** | GI, probe rays, radiance cascade intervals, transmittance | Emit segments over which occupancy is constant or bounded; do not terminate early |

These are structurally different. The traversal contract must support both.

For segment queries, the key is emitting "this whole chunk/bricklet/column region is empty" or "this region is opaque" rather than returning raw per-voxel bits. This is what PBRT's majorant iterator does conceptually.

### Invariant 7 — Coherence varies by ray type

Primary camera rays and structured probe batches are coherent. Secondary GI rays are not.

- For coherent batches: direction bucketing, octant sorting, and warp-level batching pay off
- For incoherent rays: overengineering packet traversal wastes effort; focus on per-ray efficiency instead

Do not design the entire traversal system for the incoherent case. Design for the common coherent case; handle incoherence as a second-class concern.

---

## The Three Optimization Priorities

### Priority A — Two-Level DDA

**Chunk DDA → Voxel DDA.**

This is the clearest improvement over naive full-world voxel DDA. It matches the chunk system and sparse residency naturally.

```
Level 0: Chunk DDA
  Ray steps through chunk grid using A&W
  At each chunk:
    - Not resident → skip (treat as empty)
    - chunk_flags.is_empty → skip
    - Otherwise → descend to Level 1

Level 1: Voxel DDA (inside chunk)
  Ray steps through 64³ voxel grid using A&W
  Coordinate: local = world_voxel - chunk_origin
  At each voxel:
    - Test opaque_mask bit
    - Hit → return result
    - Exit chunk → return to Level 0
```

Chunk DDA uses the same A&W formulation as voxel DDA — the algorithm is nested, not replaced.

**Future extension — Level 0.5 (bricklet DDA):**
Once `occupancy_summary` is implemented (see [[chunk-field-registry]]), add a level between chunk and voxel that tests 8³ bricklet bits before descending to per-voxel. This gives three nested levels:

```
Chunk DDA → Bricklet DDA → Voxel DDA
```

The bricklet level is optional and only worth adding if profiling shows the voxel DDA inner loop spending significant time in empty space within non-empty chunks.

---

### Priority B — Column-Aware Inner Traversal

Exploit the u64 Y-column layout inside the voxel DDA inner loop.

The column is not an optimization applied on top of A&W — it is a different traversal mode for the Y axis that replaces per-step voxel tests with bitwise scans when the ray stays in the same `(x, z)` column for multiple steps.

**Column scan for first-hit:**
```
col = opaque_mask[lx * 64 + lz]
// Mask off bits below current y
remaining = col >> ly
if remaining == 0:
    // No occupied voxels above current y in this column
    // Advance directly to column exit (next x or z step)
else:
    // First occupied voxel is at: ly + tzcnt(remaining)
    hit_y = ly + tzcnt(remaining)
    return hit at (lx, hit_y, lz)
```

This reduces an arbitrary number of empty Y steps to a single `tzcnt` instruction. For rays with a steep Y component, this can skip entire columns in one operation.

**Column scan for segment queries (transmittance/interval):**
```
col = opaque_mask[lx * 64 + lz]
// Mask to the y range [y_enter, y_exit] within this column
range_mask = col & (range_bits for [y_enter, y_exit])
if range_mask == 0:
    // Entire y range in this column is empty
    emit EmptySegment(y_enter, y_exit)
else:
    // Emit per-run segments using bit scan sequences
    ...
```

**Delay material fetch:**
In both modes, material data is only fetched after the occupancy bit confirms a solid voxel. Inside the hot traversal loop, only `opaque_mask` is accessed. `materials.palette` and `materials.index_buf` are touched at hit resolution only.

---

### Priority C — Chunk Summaries for Ray Work

Hi-Z is for the raster side. For ray traversal, the right prefilter is chunk-space facts — not camera visibility.

The summaries that help traversal are in `chunk_flags` (see [[chunk-field-registry]]):

| Flag | Ray traversal use |
|---|---|
| `is_empty` | Skip chunk entirely in Level 0 DDA |
| `has_emissive` | Include chunk in GI/probe traversal; skip if only looking for light sources |
| `is_resident` | Non-resident chunks treated as empty — no descend |

These are cheap per-chunk bits tested at Level 0, before any per-voxel work.

**What to explicitly not use as a traversal prefilter:**
- Hi-Z pyramid results
- Camera frustum cull results
- Any Product 3 (camera-visibility) data

These filter for what the camera sees. They do not filter for what rays hit. A chunk occluded from the camera may still be the target of a probe ray, shadow ray, or radiance cascade interval query.

---

## The Traversal Contract

All ray traversal in this engine must conform to one of two signatures.

### First-Hit Query

```
traceFirstHit(
    origin:    vec3<f32>,      // World-space ray origin
    direction: vec3<f32>,      // World-space ray direction (normalized)
    t_max:     f32,            // Maximum distance
) -> FirstHitResult

FirstHitResult =
    | Hit   { t: f32, voxel: vec3<i32>, face: FaceDir }
    | Miss
```

Permitted data access:
- `chunk_flags` (Level 0 skip)
- `chunk_occupancy_atlas` / `opaque_mask` (Level 1 bit test)
- `materials.palette` + `materials.index_buf` (only after hit confirmed)

Not permitted:
- Hi-Z, depth buffer, mesh geometry, any camera-visible set

---

### Segment Stream Query

```
traceSegments(
    origin:     vec3<f32>,
    direction:  vec3<f32>,
    t_start:    f32,           // Interval start
    t_end:      f32,           // Interval end
) -> Iterator<TraversalSegment>

TraversalSegment =
    | EmptySegment   { t_enter: f32, t_exit: f32 }
    | OpaqueSegment  { t_enter: f32, t_exit: f32, voxel: vec3<i32> }
```

This is the mode used by radiance cascade interval queries. Each cascade level calls `traceSegments` with its own `[t_start, t_end]` interval.

The iterator emits segments in ray-order. Empty segments can be skipped or accumulated. Opaque segments terminate a cascade interval query. Semi-transparent materials (future) would emit partial-opacity segments.

Permitted data access: same as first-hit, plus `occupancy_summary` for bricklet-level empty-segment emission.

---

## Out of Scope (Explicitly Deferred)

These are real techniques but not the right investment for the current stage:

| Technique | Why deferred |
|---|---|
| Sparse voxel octree | Two-level DDA over chunks captures most benefit without the complexity |
| Full packet traversal | Coherent rays benefit, incoherent rays don't. Profile first. |
| GPU ray reordering | Valuable for incoherent GI rays, but adds pipeline complexity. Defer until GI is running. |
| Hi-Z as traversal prefilter | Architecturally incorrect — conflates camera-visibility with ray-relevance |
| BVH over voxels | Wrong data structure for dense occupancy grids |

---

## What Needs to Be Built

| Component | Status | Blocks |
|---|---|---|
| Chunk DDA (Level 0) | Not implemented | Everything |
| Voxel DDA (Level 1) | Not implemented | First-hit, segment stream |
| `chunk_flags` GPU buffer | Not implemented | Level 0 skip |
| `occupancy_summary` GPU buffer | Not implemented | Bricklet skip (Priority A extension) |
| Column-aware Y scan | Not implemented | Priority B |
| `traceFirstHit` WGSL | Not implemented | Picking, shadow |
| `traceSegments` WGSL | Not implemented | GI, radiance cascades |

The build order is: `chunk_flags` → Level 0 DDA → Level 1 DDA → `traceFirstHit` → `traceSegments` → column-aware inner loop → `occupancy_summary` → bricklet level.

---

## See Also

- [[chunk-field-registry]] — fields consumed by traversal (`opaque_mask`, `chunk_flags`, `occupancy_summary`)
- [[layer-model]] — why traversal (Product 1) must not be filtered by camera-visibility (Product 3)
- [[pipeline-stages]] — Stage R-6 (radiance cascade build) is the first consumer of `traceSegments`
- [[../woo/Amanatides_and_Woo]] — DDA algorithm reference
