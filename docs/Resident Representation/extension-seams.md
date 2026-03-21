# Extension Seams

**Type:** spec
**Status:** current
**Date:** 2026-03-21

How future features integrate with the canonical voxel runtime without becoming bolt-ons.

This document is the architectural principles layer. Before any new system is designed, run it through this framework first.

---

## The Rule on the Wall

> Choose the canonical data model based on the queries the runtime must answer repeatedly, not on the convenience of any one content-generation path.

That is the design criterion. Everything else follows from it.

The canonical form for this engine is:

**Chunked dense-or-bitpacked occupancy with small local summaries.**

Why this form survives contact with every system that needs to touch voxel data:

| System | Relationship |
|---|---|
| Density field evaluation | can write it |
| GPU voxelizer | can write it |
| Brush edits | can write it |
| Greedy meshing | can read it |
| A&W traversal | can read it |
| Chunk streaming | can manage it |
| World-space GI | can read it |
| GPU residency logic | can reason about it |

No other candidate data model satisfies this many systems without requiring each system to do significant translation work. That breadth of native compatibility is the signal that the form is right.

The "convenience of content generation" failure mode looks like this: the voxelizer produces CompactVoxel[] efficiently, so CompactVoxel[] starts to feel like the natural center. But CompactVoxel[] is convenient for one producer. Chunked occupancy is useful to every consumer. Those are different criteria, and the runtime should optimize for the consumer side.

---

## The One-Sentence Rule

> A future optimization is natively integrated when it can be expressed as a derived query layer over canonical chunk truth. It is a bolt-on when the chunk truth is redesigned to impersonate that optimization.

---

## The Smell Test

A future feature feels **bolted on** when it requires any of:
- A second authoritative representation of the world
- Core spatial metadata duplicated in an incompatible form
- Every producer to know about a downstream optimization
- Chunk/runtime data that depends on one rendering path's quirks

A future feature feels **native** when it can be expressed as one of:
- A new derived view over canonical voxel data
- A new summary layer over canonical voxel data
- A new consumer of canonical voxel data
- A new producer writing the same canonical voxel contract

That is the whole game.

---

## The Architectural Test

For any proposed feature, answer three questions in order:

**1. Is it authoritative world truth?**
If yes, be extremely reluctant. The set of authoritative fields is small by design. Growing it adds constraints on every producer, every consumer, and every migration path. See [chunk-field-registry](chunk-field-registry.md).

**2. Is it a derived acceleration layer?**
If yes, it is probably fine. Derived structures can be added, discarded, rebuilt, and version-gated without touching the core contract.

**3. Is it view-dependent or query-dependent?**
If yes, it should almost certainly not become authoritative. View-dependent structures (depth buffers, Hi-Z pyramids, visibility sets, camera-relative representations) live downstream of the canonical world. They respond to the world; they do not define it.

---

## The Six Sacred Invariants

These are the stable properties that make future additions feel native. A feature that violates any of these is not an extension — it is a redesign.

**Invariant 1 — Chunked occupancy is world truth.**
Everything else is derived. No competing authoritative world model.

**Invariant 2 — Chunks are independently queryable and independently dirty.**
Edits, async rebuild, traversal descent, streaming, and residency all work per-chunk, with no global lock and no dependency on other chunks (except boundary marking). Any feature that requires coordinated multi-chunk dirty state is violating this invariant.

**Invariant 3 — Chunk-local occupancy queries are cheap.**
The u64 column layout exists to make this true. Any feature that requires restructuring chunk-local storage for its own traversal preferences should instead define a derived representation built from the existing layout.

**Invariant 4 — World → chunk → local coordinate mapping is stable and cheap.**
Shifts and masks. Power-of-two. Euclidean division for negatives. This lets traversal layers slot in cleanly. Features that require a different coordinate system must build their own mapping over the existing one, not replace it.

**Invariant 5 — Authoritative data is producer-agnostic.**
The voxelizer, density evaluator, brush edit, simulation step, and any future producer all write the same canonical voxel contract. No producer gets to invent a special runtime format that only it understands.

**Invariant 6 — View-dependent structures are downstream.**
Meshes, depth pyramids, Hi-Z, visibility sets, packet schedules, camera-relative probes — all of these are consumers and derivatives. They may influence streaming priority or rebuild scheduling via hints, but they must not become authoritative or dictate how chunks are stored.

---

## The Four-Layer Architecture

Future features slot into exactly one layer. Smearing across layers is the warning sign.

```
Layer 1 — Canonical voxel world
  Per chunk: opaque_mask, materials, coord, data_version, state
  Written by producers. Never derived. Never discarded without re-population.

Layer 2 — Reusable spatial summaries
  chunk_flags, occupancy_summary, aabb, future emissive masks, subregion hierarchies
  Derived from Layer 1. Stable enough to be shared across multiple consumers.
  Built once per dirty cycle; reused by many downstream systems.

Layer 3 — Query engines / traversal
  Greedy mesher, scalar DDA, packet DDA, probe tracing, physics, streaming heuristics
  Consume Layer 1 + Layer 2. Produce layer-4 artifacts or query results.
  Do not store persistent world state.

Layer 4 — View-dependent products
  Chunk meshes, depth buffer, Hi-Z pyramid, visibility lists, cascade atlases, shading inputs
  Per-frame or per-render. Camera or query specific. Never feed back into Layer 1.
```

A feature that can be stated as "I need a new Layer 2 summary" or "I need a new Layer 3 consumer" is native.

A feature that requires "Layer 1 should work differently because of my Layer 4 need" is a bolt-on.

---

## Healthy Integration Patterns

**"I need a new derived summary."**
Build it in Layer 2. It reads from Layer 1, rebuilds when chunks are dirtied, lives alongside existing summaries.
Examples: sub-brick occupancy mip, emissive mask, occupied AABB, cone visibility hints.

**"I need a new consumer."**
Build it in Layer 3. It implements the `traceFirstHit` / `traceSegments` contract or the mesh output contract. Does not change Layer 1.
Examples: world-space probe tracing, packet traversal, software shadows, streaming predictor, physics query.

**"I need a new producer."**
Write to the same canonical chunk contract. Does not know about any specific consumer.
Examples: density evaluator, brush editing, CSG tool, imported sparse scene, simulation step.

---

## Unhealthy Integration Patterns

These are the warning signs that a feature has entered the wrong layer.

| Pattern | Diagnosis |
|---|---|
| "I need the chunk format to stop being chunked." | Violates Invariant 1. Feature wants a competing world model. |
| "All producers must emit my special hierarchy." | Violates Invariant 5. Feature is colonizing the producer interface. |
| "World truth should become camera-relative." | Violates Invariant 6. View-dependent structure is trying to become authoritative. |
| "Mesh buffers should be authoritative." | Violates Invariant 6. Layer 4 artifact demanding Layer 1 status. |
| "Hi-Z should filter which chunks traversal queries." | Mixes Product 3 (camera-visibility) with Product 1 (world-space ray work). See [layer-model](layer-model.md). |
| "Every edit must synchronously update my acceleration structure." | Violates Invariant 2. Feature is taking a global dependency on dirty state. |
| "CompactVoxel[] is the stable API between systems." | See below. |

---

## The CompactVoxel[] Anti-Pattern

**Do not let CompactVoxel[] become the philosophical center of the engine.**

CompactVoxel[] is a transport format — a flat `[vx, vy, vz, material, ...]` array used to move voxel data from a producer (voxelizer, editor) into the canonical chunk runtime. It is good at that job. It is a portable, debuggable, CPU-friendly interchange format.

It is not:
- The canonical world representation
- A stable query API
- A runtime data structure
- The thing consumers should build on

**The failure mode** is inertia. CompactVoxel[] is the most visible seam in the current pipeline, so systems built quickly tend to grow roots there. Over time it accumulates consumers, gets treated as a stable interface, and the chunk structure becomes a dependent of it rather than its destination.

**The correct mental model:**

```
CompactVoxel[] is a courier.
Chunk occupancy is the city.

The courier delivers the message and leaves.
The city persists, handles queries, and evolves.
```

Consumers must query chunk occupancy, not the courier format. If a consumer is reading CompactVoxel[] at runtime, it is reading a transit artifact, not the world.

**In the GPU-resident target, this becomes structural.** The voxelizer writes directly into GPU chunk occupancy slots. CompactVoxel[] becomes an optional CPU-side debug path and fallback ingest route — not the main highway. Anything built on CompactVoxel[] as a runtime dependency gets stranded when that transition happens.

**The rule:** CompactVoxel[] is valid for:
- CPU-side ingest (current, transitional)
- Serialization / deserialization
- Debugging and validation tooling
- Fallback paths on non-GPU platforms

CompactVoxel[] is not valid for:
- Runtime world queries
- Traversal inputs
- GI or probe data
- Any consumer that will outlive the CPU ingest path

---

## Applying This to Specific Future Features

### Sparse Voxel Octrees

**Layer:** 2 (derived traversal hierarchy) or 3 (query consumer).

An SVO is a valid derived representation over some region set, or a secondary far-field representation. It fits naturally as a hierarchical traversal structure built from chunk occupancy.

It is not a replacement for Layer 1. Attempting to make canonical world truth octree-first would break edits, dirty tracking, palette materials, streaming, and chunk locality — all of which are designed around the fixed 64³ chunk.

**Healthy seam:**
```
canonical chunks
  → optional SVO over chunk occupancy    (Layer 2)
  → SVO traversal for far-field queries  (Layer 3)
```

### Packet Traversal / Ray Sorting

**Layer:** 3 (execution strategy over existing traversal contract).

This is a scheduler, not a world representation. It should consume the same `traceFirstHit` / `traceSegments` contract as scalar traversal, with a different dispatch and memory access pattern.

If packet traversal would require redesigning the chunk format, something is architecturally wrong.

**Healthy seam:**
```
same Layer 1 + Layer 2 data
  → same traversal contract
  → packet-organized dispatch (Layer 3 execution mode)
```

### Camera Hi-Z

**Layer:** 4 (view-dependent product).

Hi-Z is derived from raster depth. It tells you what the *camera* sees this frame. Its only acceptable influence on upper layers is soft hints:
- Streaming priority
- Rebuild priority
- Debug / profiling feedback

Hi-Z must not gate which chunks are resident, which chunks are traversed by rays, or how chunks are stored. See [layer-model](layer-model.md) for why camera-visibility and ray-relevance are orthogonal.

---

## The Extensibility Test

Before implementing any new feature, fill out this table:

| Question | Answer | Assessment |
|---|---|---|
| What authoritative chunk data does it read? | list fields | If it needs new authoritative fields, justify carefully |
| What Layer 2 summaries does it need? | list or "none yet" | If missing, plan to build them as Layer 2 additions |
| What does it produce? | query result / Layer 2 summary / Layer 4 artifact | Should be exactly one layer |
| Update semantics | per-frame / per-dirty-chunk / on-demand | Should not require synchronous global updates |
| Who owns it? | core runtime / traversal module / renderer / debug | Must be one owner; shared ownership is a seam smell |
| Can it be removed without touching Layer 1? | yes / no | Answer must be yes |

A feature that passes all six rows is architecturally native. A feature that fails any row needs to be redesigned before implementation begins.

---

## The Three-Column Classification

Where do current and planned structures sit?

| Authoritative (Layer 1) | Derived Reusable (Layer 2) | View / Query Specific (Layer 3–4) |
|---|---|---|
| `opaque_mask` | `occupancy_summary` | Greedy mesh buffers |
| `materials.palette` | `chunk_flags` | Hi-Z pyramid |
| `materials.index_buf` | `aabb` | Depth buffer |
| `coord` | (future) emissive mask | Cascade atlas |
| `data_version` | (future) subregion mip | Indirect draw args |
| `state` | | Visibility lists |
| | | Per-frame camera uniforms |

Rules for this table:
- Nothing moves left (from derived to authoritative) without a compelling, documented case
- Nothing in the right column should influence the left column's structure
- The middle column grows as new consumers prove they need shared summaries
- The right column is always per-frame or per-query — never cached as world truth

---

## The Junk Drawer Warning

The mistake is "future-proofing" by stuffing every possible acceleration concept into the chunk schema itself. That creates a junk drawer with pretensions: a structure that is nominally canonical but in practice shaped by every optimization anyone has ever thought about.

The right response to a speculative future need is not to add it to Layer 1. It is to verify that Layer 1 exposes the right attachment points for a future Layer 2 addition.

The attachment points are:
- `opaque_mask` is readable (traversal can descend here)
- `materials` is readable on confirmed hit (consumers get material data)
- `chunk_flags` gives cheap coarse-skip signals (consumers skip empty chunks)
- `data_version` + dirty tracking gives incremental rebuild hooks (summaries rebuild on dirty)
- `coord` gives world-space anchoring (everything can locate itself)

Those five attachment points are enough for most imaginable extensions. If a proposed feature cannot be expressed in terms of those five attachment points plus its own Layer 2 additions, the feature design has a problem.

---

## See Also

- [chunk-field-registry](chunk-field-registry.md) — explicit classification of every current field
- [layer-model](layer-model.md) — the three-product architecture (world-space / surface / camera-visibility)
- [chunk-contract](chunk-contract.md) — edit semantics, residency protocol, and what gets invalidated on edit
- [traversal-acceleration](traversal-acceleration.md) — Layer 3 query contract (`traceFirstHit`, `traceSegments`)
