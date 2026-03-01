> **SUPERSEDED** — This document is a historical problem-space framing from before Architecture B was decided.
> The architectural principle it established is now the foundation of the integration.
> Content preserved in: `philosophy.md` (canonical store principle, architectural direction).
> This file is retained as a historical record only.

---

# Voxelizer + Greedy Mesher Unification Report (Draft v0)

Date: February 19, 2026
Status: Initial draft ("beginnings" report)
Audience: Rendering/engine architecture and module maintainers

## Document Contract

1. Type: Descriptive (problem-space framing).
2. Primary use: Understand mismatches and integration risk domains.
3. Downstream prescriptive doc:
   - `docs/greedy-meshing-docs/voxelizer-greedy-native-migration-outline.md`
4. Historical companion:
   - `docs/greedy-meshing-docs/original-reasoning-sparse-brick-occupancy-batches.md`
5. Navigation hub:
   - `docs/greedy-meshing-docs/voxelizer-greedy-program-map.md`

## 1. Why this report exists

The project direction is clear: greedy-mesh chunk rendering is the long-term default path for voxel geometry.  
Today, the voxelizer and greedy mesher still expose different assumptions and interfaces, which creates integration friction.

This report documents the major complexity areas involved in unifying these systems and frames a migration-friendly path.

## 2. Scope

In scope:

1. Interface and data-model mismatches between voxelizer outputs and greedy chunk meshing inputs.
2. Material/palette concerns needed for a "hyper-palettable" chunk pipeline.
3. Runtime and lifecycle mismatches (snapshot output vs incremental chunk updates).
4. Worker/WASM boundary implications.

Out of scope (for this draft):

1. Final API spec details (this draft proposes direction, not final signatures).
2. Full implementation schedule with estimates.
3. Rendering-stage occlusion implementation details (covered separately in Hi-Z report).

## 3. Current system snapshot

### 3.1 Voxelizer side (current emphasis)

Current voxelizer flow and adapters are oriented around sparse bricks and render-adjacent outputs:

1. Sparse output shape (`brick_dim`, `brick_origins`, `occupancy`) in `crates/voxelizer/src/core.rs:143`.
2. WASM voxelizer exports sparse/chunked/positions outputs in `crates/wasm_voxelizer/src/lib.rs`.
3. JS adapter exposes sparse/positions expansion utilities in `packages/voxelizer-js/src/index.ts:9` and `packages/voxelizer-js/src/index.ts:300`.
4. Web module turns voxelizer results into viewer outputs (`kind: "voxels"`/`kind: "lines"`) in `apps/web/src/modules/wasmVoxelizer/runCore.ts:153` and `apps/web/src/modules/wasmVoxelizer/runCore.ts:300`.

### 3.2 Greedy mesher side (target emphasis)

Greedy mesher architecture is chunk-state and material-aware:

1. Fixed padded chunk core: `CS_P=64`, `CS=62` in `crates/greedy_mesher/src/core.rs:12`.
2. Material model: `MaterialId = u16` in `crates/greedy_mesher/src/core.rs:3`.
3. Palette-backed chunk storage in `BinaryChunk.materials` (`PaletteMaterials`) at `crates/greedy_mesher/src/core.rs:54`.
4. Chunk lifecycle/update system in `ChunkManager` (`set_voxel`, `set_voxels_batch`, `populate_dense`, `update`) in `crates/greedy_mesher/src/chunk/manager.rs:170`, `crates/greedy_mesher/src/chunk/manager.rs:214`, `crates/greedy_mesher/src/chunk/manager.rs:787`, and `crates/greedy_mesher/src/chunk/manager.rs:696`.
5. JS worker/client path already supports chunk-manager orchestration in `apps/web/src/modules/wasmGreedyMesher/workers/mesher.worker.ts:509`.

## 4. Core complexity domains

## 4.1 Data shape mismatch: sparse bricks vs chunk-native updates

Voxelizer emits sparse brick occupancy batches; greedy mesher consumes chunk-centric voxel/material state.

Complexity:

1. Brick boundaries do not necessarily align with greedy chunk boundaries.
2. A single brick can touch multiple chunks at chunk edges.
3. Conversion requires deterministic mapping into chunk-local coordinates (+padding model).

Why this matters:

1. Poor mapping logic creates chunk boundary artifacts.
2. Non-deterministic mapping destabilizes chunk versioning and rebuild behavior.

## 4.2 Material semantics mismatch

Voxelizer currently has the concept of per-triangle material input (`MeshInput.material_ids`) but the wasm path commonly sets `material_ids: None` in key entrypoints (`crates/wasm_voxelizer/src/lib.rs:205`, `crates/wasm_voxelizer/src/lib.rs:745`).

Greedy mesher requires reliable per-voxel material IDs for merge correctness and palette efficiency.

Complexity:

1. Need a stable mapping from source triangle/material domain to `u16 MaterialId`.
2. Need deterministic per-voxel winner policy when multiple triangles contribute.
3. Need consistency across chunk boundaries and across updates.

## 4.3 Palette strategy mismatch

Greedy chunk storage is palette-compressed by design; current voxelizer outputs are not directly palette-first at the module boundary.

Complexity:

1. If voxelizer output arrives as raw occupancy + ad hoc attributes, chunk ingest will incur repeated palette churn.
2. High churn causes repacks and CPU overhead in `PaletteMaterials`.
3. Unstable material indexing hurts cacheability and incremental diffing.

Required direction:

1. Introduce stable material identity and chunk-local palette encoding strategy during conversion.

## 4.4 Lifecycle mismatch: snapshot render output vs incremental world updates

Current voxelizer module returns output snapshots for immediate rendering (`runCore.ts` output arrays), while greedy chunk manager is built for persistent mutable world state.

Complexity:

1. Snapshot mode discards update intent and change locality.
2. Chunk manager expects edits/deltas over time, version tracking, and dirty propagation.
3. One-shot rebuild semantics can mask long-term consistency bugs.

Integration implication:

1. Voxelizer should become a producer of chunk edits/deltas, not a direct render payload generator for the default path.

## 4.5 Coordinate and boundary policy mismatch

Greedy chunk manager uses voxel-space addressing with chunk-local transforms and explicit boundary synchronization.

Complexity:

1. Float world coordinates in module outputs are lossy for authoritative chunk updates.
2. Conversion must preserve integer voxel coordinates and exact chunk boundary semantics.
3. Any rounding differences will produce cross-chunk seams and stale padding synchronization.

## 4.6 WASM/worker boundary cost and ownership

Both systems already rely on worker/WASM interop. Unification can either reduce or increase marshalling overhead.

Complexity:

1. Sending large expanded position arrays (old pattern) is expensive and no longer aligned with target architecture.
2. Sending compact chunk-delta payloads is better but requires protocol redesign.
3. Ownership of conversion logic (TS vs Rust) impacts memory copying and debugging complexity.

## 4.7 Dual-path risk during migration

You will likely run voxel-render legacy and greedy-render target in parallel temporarily.

Complexity:

1. Two code paths increase bug surface and observability burden.
2. Mismatched diagnostics can hide semantic divergence.
3. Team can accidentally optimize the legacy path if clear guardrails are not set.

## 5. Guiding architectural principle

Unification should optimize for the target system, not preserve convenience in the legacy path.

Practical rule:

1. Treat voxelizer as data producer for chunk/material state.
2. Treat greedy chunk manager/mesher as the authoritative rendering ingestion path.
3. Keep legacy voxel visualization as debug/bridge only.

## 6. Early target contract direction

Initial contract direction (conceptual):

1. `voxelizer -> chunk deltas` payload:
   - chunk coordinate
   - list/packing of local voxel edits (or dense local block)
   - material ID payload aligned with `u16 MaterialId`
2. `chunk manager` ingests payload and performs:
   - dirty marking
   - boundary propagation
   - budgeted rebuild/update
3. `viewer` renders mesh outputs from chunk manager only for default voxel geometry path.

This removes "expanded voxel position arrays" from the default path.

## 7. Key unresolved questions

These should be answered before final API lock:

1. Where should brick->chunk conversion live (Rust-side preferred vs TS bridge)?
2. What is the canonical global material registry and ID assignment policy?
3. Do we want chunk-local palettes in transport, or only global IDs in transport and local palette creation in chunk storage?
4. What is the deterministic tie-break policy for material assignment when voxelizer hits overlap?
5. Which migration checkpoints define "legacy path can no longer block architecture decisions"?

## 8. Suggested next report increment

For v1 of this report:

1. Add concrete wire format proposal (`VoxelizerChunkDelta` family).
2. Add exact ownership matrix (voxelizer crate, wasm bindings, worker, module host, chunk manager).
3. Add migration stages with explicit de-prioritization gates for legacy voxel-render outputs.
4. Add success metrics (palette stability, delta size, rebuild cost, correctness checks at chunk boundaries).

---

This draft is intentionally focused on complexity framing and architectural direction. It should be followed by a concrete contract proposal and phased migration plan.
