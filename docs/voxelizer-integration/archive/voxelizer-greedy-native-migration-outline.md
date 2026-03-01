> **SUPERSEDED** — This document proposes Architecture A migration phases and a `VoxelizerChunkDeltaBatch` wire format
> that is not used in Architecture B. Architecture B is the current design. See `docs/voxelizer-integration/`.
> Project goals and success criteria preserved in: `design/requirements.md`, `adr/0009-architecture-b.md`.
> This file is retained as a historical record. Do not implement from it.

---

# Voxelizer -> Greedy-Native Migration Outline

Date: February 20, 2026
Status: Proposed outline
Audience: Voxelizer, WASM bindings, worker/runtime, and greedy-mesher maintainers

## Document Contract

1. Type: Prescriptive (primary implementation contract).
2. Primary use: Decide what to build and in what order.
3. Upstream context:
   - `docs/greedy-meshing-docs/voxelizer-greedy-mesher-unification-report.md`
   - `docs/greedy-meshing-docs/original-reasoning-sparse-brick-occupancy-batches.md`
4. Adjacent constraints:
   - `docs/greedy-meshing-docs/voxelizer-materials-state-requirements-architecture-report.md`
   - `docs/culling/hiz-occlusion-culling-report.md`
5. Navigation hub:
   - `docs/greedy-meshing-docs/voxelizer-greedy-program-map.md`

## 1. Decision summary

Keep the voxelizer core. Replace the output contract.

Specifically:

1. Reuse the existing voxelization engine (`crates/voxelizer`).
2. Add a new greedy/chunk-native output path from voxelizer.
3. Route that output directly into chunk manager ingestion.
4. Freeze legacy sparse-preview adapters as debug/compat only.

This preserves proven GPU voxelization logic while aligning runtime behavior with the greedy chunk system.

## 2. Project goals

## 2.1 Primary goals

1. Make voxelizer output natively consumable by greedy chunk storage.
2. Eliminate default-path position expansion (`x,y,z` per voxel floats) for rendering.
3. Preserve bounded-memory batch behavior from sparse brick processing.
4. Support stable per-voxel material IDs for greedy merges and palette efficiency.
5. Keep runtime centered on `ChunkManager` lifecycle (`dirty -> rebuild -> swap -> evict`).

## 2.2 Secondary goals

1. Reduce JS-side glue complexity from experimental preview-era adapters.
2. Minimize WASM boundary chatter by favoring chunk-level bulk ingestion.
3. Keep a feature-flagged fallback to current sparse preview during migration.

## 2.3 Non-goals

1. Rewriting voxelizer triangle intersection math.
2. Replacing GPU sparse dispatch architecture.
3. Immediate deletion of all legacy preview code on day one.

## 3. Current constraints and ground truth

1. Voxelizer emits sparse brick output (`SparseVoxelizationOutput`) with bitpacked occupancy:
   `crates/voxelizer/src/core.rs:143`.
2. Sparse chunking is dispatch-batching, not greedy world chunks:
   `crates/voxelizer/src/gpu/sparse.rs:30`, `crates/voxelizer/src/gpu/sparse.rs:98`.
3. Current wasm voxelizer paths mostly pass `material_ids: None`:
   `crates/wasm_voxelizer/src/lib.rs:207`, `crates/wasm_voxelizer/src/lib.rs:745`.
4. Current JS voxelizer adapter expands sparse data to positions for rendering:
   `packages/voxelizer-js/src/index.ts:312`, `packages/voxelizer-js/src/index.ts:351`.
5. Greedy system wants chunk-native material-aware voxel edits:
   `crates/greedy_mesher/src/chunk/manager.rs:214`.
6. Greedy chunk coordinate system uses `CS=62` usable size (+padding in storage):
   `crates/greedy_mesher/src/core.rs:16`, `crates/greedy_mesher/src/chunk/coord.rs:82`.

## 4. Reuse / modify / retire matrix

| Area | Action | Why |
|------|--------|-----|
| `crates/voxelizer` sparse GPU core | Reuse | Core voxelization is mature and already memory-bounded |
| Brick CSR generation (`build_brick_csr`) | Reuse | Correct sparse spatial indexing exists |
| Sparse batching logic (`max_bricks_per_dispatch`) | Reuse | Required for safe cross-device memory behavior |
| `crates/wasm_voxelizer` output surface | Modify heavily | Must emit chunk-native deltas instead of preview payloads |
| Material flow in voxelizer entrypoints | Modify | Needs deterministic `MaterialId` propagation |
| `packages/voxelizer-js` brick paging/position expansion | Retire from default path | Preview-era pattern, not greedy-native ingestion |
| `apps/web/src/modules/wasmVoxelizer/runCore.ts` preview orchestration | Split | Keep debug mode; default route to chunk manager |
| Greedy chunk manager ingestion API | Extend | Add bulk chunk-delta apply API |

## 5. Target contract: greedy-native voxelizer output

## 5.1 Contract intent

Output should represent voxel edits grouped by greedy chunk coordinates, not brick coordinates and not render-ready floats.

## 5.2 Proposed transport schema (v1)

Rust-side conceptual types:

```rust
pub struct VoxelizerChunkDeltaBatch {
    pub voxel_size: f32,
    pub chunk_size: u32,              // expected 62 (greedy usable size)
    pub chunks: Vec<VoxelizerChunkDelta>,
    pub stats: VoxelizerChunkDeltaStats,
}

pub struct VoxelizerChunkDelta {
    pub chunk_coord: [i32; 3],        // greedy chunk-space coordinate
    pub local_xyz: Vec<u8>,           // packed triplets [x0,y0,z0,...], each 0..61
    pub materials: Vec<u16>,          // MaterialId per local voxel entry
}

pub struct VoxelizerChunkDeltaStats {
    pub bricks_processed: u32,
    pub voxels_emitted: u32,
    pub chunks_touched: u32,
}
```

TypeScript-side shape:

```ts
type VoxelizerChunkDeltaBatch = {
  voxelSize: number;
  chunkSize: number; // 62
  chunks: Array<{
    chunkCoord: Int32Array; // length 3
    localXYZ: Uint8Array;   // length 3*N
    materials: Uint16Array; // length N
  }>;
  stats: {
    bricksProcessed: number;
    voxelsEmitted: number;
    chunksTouched: number;
  };
};
```

## 5.3 Why this schema

1. Chunk-keyed updates match `ChunkManager` ownership model.
2. `local_xyz` avoids world-float precision ambiguity.
3. `u16` materials match `MaterialId` directly.
4. Keeps payload sparse while remaining ingest-friendly.

## 6. Conversion model: sparse bricks -> chunk deltas

Conversion is best done in Rust (inside wasm voxelizer boundary), not in TS.

Algorithm:

1. Iterate each sparse brick origin and occupancy words.
2. For each occupied local voxel in brick:
3. Compute global voxel index.
4. Compute `chunk_coord = floor_div(global, 62)`.
5. Compute `local = rem_euclid(global, 62)`.
6. Emit `(local_xyz, material)` into chunk bucket.
7. Deduplicate policy for collisions (last-writer or deterministic material priority).
8. Serialize chunk buckets into `VoxelizerChunkDeltaBatch`.

This preserves sparse batching internals while exposing greedy-native data externally.

## 7. Ingestion model on greedy side

Add a bulk API instead of many `set_voxel_at` calls across JS/WASM boundary.

Current primitives:

1. `set_voxel_at` and `set_voxels_batch` exist:
   `crates/wasm_greedy_mesher/src/lib.rs:480`, `crates/wasm_greedy_mesher/src/lib.rs:486`.
2. Chunk manager batch edit groups by chunk:
   `crates/greedy_mesher/src/chunk/manager.rs:214`.

Proposed addition:

1. `WasmChunkManager.apply_chunk_deltas(...)` with chunk-grouped arrays.
2. Rust performs direct per-chunk raw sets + single version increment per touched chunk.
3. Dirty marking and neighbor sync remain in chunk manager domain.

## 8. Migration phases

## Phase 0: Baseline and guardrails

Deliverables:

1. Establish baseline metrics for current sparse-preview path (latency, bytes transferred, voxel count).
2. Add feature flags:
   - `voxelizerOutputMode = legacy_sparse | chunk_delta_v1`
   - `voxelizerRenderMode = debug_preview | greedy_default`

## Phase 1: Contract scaffolding (no behavior change)

Deliverables:

1. Add shared TS/Rust schema definitions for `VoxelizerChunkDeltaBatch`.
2. Add placeholder worker message types for chunk-delta ingestion.
3. Keep old path active.

Primary files:

1. `apps/web/src/modules/wasmGreedyMesher/workers/chunkManagerTypes.ts`
2. `apps/web/src/modules/wasmGreedyMesher/workers/chunkManagerClient.ts`
3. `crates/wasm_voxelizer/src/lib.rs` (new exported type path)

## Phase 2: Rust-side brick->chunk conversion and export

Deliverables:

1. Implement conversion from sparse outputs to chunk deltas inside wasm voxelizer.
2. Add new wasm export (example): `voxelize_triangles_chunk_deltas`.
3. Keep existing sparse exports for compatibility.

Primary files:

1. `crates/wasm_voxelizer/src/lib.rs`
2. New helper module under `crates/wasm_voxelizer/src/` for conversion logic
3. Optional shared helpers in `crates/voxelizer/src/` if reuse is needed

## Phase 3: Greedy chunk manager bulk ingestion API

Deliverables:

1. Add `apply_chunk_deltas` API to wasm greedy bindings.
2. Route ingestion to chunk-manager internals using chunk-grouped raw writes.
3. Ensure versioning and dirty-neighbor marking semantics are preserved.

Primary files:

1. `crates/wasm_greedy_mesher/src/lib.rs`
2. `crates/greedy_mesher/src/chunk/manager.rs`

## Phase 4: Worker/runtime integration

Deliverables:

1. Add worker message `cm-apply-chunk-deltas`.
2. Integrate voxelizer output path directly into chunk manager flow.
3. Remove default dependence on `flattenBricksFromChunks`/`buildPositionsForBricks`.

Primary files:

1. `apps/web/src/modules/wasmGreedyMesher/workers/chunkManagerTypes.ts`
2. `apps/web/src/modules/wasmGreedyMesher/workers/mesher.worker.ts`
3. `apps/web/src/modules/wasmGreedyMesher/workers/chunkManagerClient.ts`
4. `packages/voxelizer-js/src/index.ts`
5. `apps/web/src/modules/wasmVoxelizer/runCore.ts`

## Phase 5: Default flip + legacy freeze

Deliverables:

1. Make `chunk_delta_v1 + greedy_default` the default.
2. Keep legacy sparse preview behind explicit debug flag only.
3. Add deprecation warnings for preview-only adapters.

## Phase 6: Cleanup

Deliverables:

1. Remove dead JS pathways that are no longer used in default runtime.
2. Retain only minimal debug tooling for sparse inspection.
3. Update docs and module contracts to reflect chunk-native architecture.

## 9. Detailed success criteria

Functional:

1. Same or better voxel surface correctness vs current sparse preview baseline.
2. Chunk boundary correctness with no seam artifacts on 62-voxel edges.
3. Deterministic material assignment under overlapping contributions.

Performance:

1. Lower JS allocation volume vs position expansion path.
2. Reduced worker message payload size for dense scenes.
3. No regression in GPU voxelization stability across large inputs.

Operational:

1. Chunk manager debug stats remain coherent (`dirty`, `swapped`, `evicted`).
2. No increase in version conflicts from ingestion changes.

## 10. Testing plan

## 10.1 Unit tests

1. Brick->chunk mapping with negative and positive voxel coordinates.
2. Local coordinate conversion invariants (`0..61` range).
3. Collision resolution determinism.

## 10.2 Integration tests

1. End-to-end voxelize -> chunk-delta -> chunk-manager update -> mesh extraction.
2. Boundary stress tests that hit all six chunk faces.
3. Material palette stress tests with high material cardinality.

## 10.3 Regression tests

1. Compare meshed triangle counts and topology vs known fixtures.
2. Validate no silent drop of sparse occupied voxels during conversion.

## 11. Risks and mitigations

Risk: Material semantics are currently under-specified.  
Mitigation: lock a `MaterialId` mapping policy before output contract freeze.

Risk: Conversion duplicates or drops boundary voxels.  
Mitigation: add voxel-count conservation assertions in debug builds.

Risk: Worker protocol churn destabilizes module UX.  
Mitigation: dual-path feature flag period with staged rollout.

## 12. Immediate next implementation tasks

1. Add `chunk_delta_v1` type definitions to worker protocol.
2. Implement Rust converter in `crates/wasm_voxelizer/src/lib.rs` behind new export.
3. Add `WasmChunkManager.apply_chunk_deltas` and wire into worker.
4. Add feature flag in `apps/web/src/modules/wasmVoxelizer/runCore.ts` to switch from legacy sparse preview to greedy-native path.

---

This outline intentionally keeps voxelizer compute internals stable while moving the integration boundary to chunk-native contracts. That gives the project a clean migration path without discarding proven core logic.
