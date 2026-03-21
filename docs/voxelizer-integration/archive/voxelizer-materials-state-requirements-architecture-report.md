**Type:** legacy
**Status:** legacy

> **SUPERSEDED** — This document contains an Architecture A implementation plan (§4) that is not used.
> The MAT-REQ requirements (§3) and material pipeline analysis remain valuable.
> Content preserved in: `design/requirements.md` (MAT-REQ-01 through MAT-REQ-14), `spec/material-pipeline.md`.
> This file is retained as a historical record. Do not implement from the §4 implementation plan.

---

# Voxelizer Materials: Current State, Requirements, and Architecture Update Plan

**Date:** 2026-02-20
Status: Proposed report
Audience: Voxelizer, greedy-mesher, worker/runtime maintainers

## Document Contract

1. Type: Prescriptive+descriptive (materials-focused integration report).
2. Primary use: Material support scope, constraints, and implementation direction.
3. Upstream context:
   - `docs/greedy-meshing-docs/voxelizer-greedy-mesher-unification-report.md`
   - `docs/greedy-meshing-docs/voxelizer-greedy-native-migration-outline.md`
4. Navigation hub:
   - `docs/greedy-meshing-docs/voxelizer-greedy-program-map.md`

## 1. Executive summary

Material support is not solved end-to-end for voxelizer -> greedy integration.

Current reality:

1. Voxelizer internally tracks occupancy + owner provenance + debug color.
2. Greedy mesher already supports true `MaterialId` (`u16`) and material-aware merges.
3. The boundary between them does not currently carry canonical material semantics.

Decision direction:

1. Keep sparse-brick voxelization internals for memory safety.
2. Add material-aware chunk-native output contract at the voxelizer/WASM boundary.
3. Feed chunk manager with deterministic `MaterialId` deltas.

## 2. Current state (actual code state)

## 2.1 Voxelizer data model and compute

1. `MeshInput` includes optional `material_ids`, but only validates length.
   - `crates/voxelizer/src/core.rs:88`
   - `crates/voxelizer/src/core.rs:93`
2. Sparse output schema does not include material IDs; it includes:
   - `occupancy`
   - `owner_id` (triangle provenance)
   - `color_rgba` (debug color hash)
   - `crates/voxelizer/src/core.rs:143`
3. GPU voxelizer writes `owner_id` as smallest intersecting triangle index and `color_rgba` as hash(owner).
   - `crates/voxelizer/src/gpu/shaders.rs:225`
   - `crates/voxelizer/src/gpu/shaders.rs:245`
   - `crates/voxelizer/src/gpu/shaders.rs:248`
4. CPU fallback mirrors the same owner/color behavior.
   - `crates/voxelizer/src/reference_cpu.rs:113`
   - `crates/voxelizer/src/reference_cpu.rs:128`

Conclusion: voxelizer currently provides provenance/debug attributes, not gameplay/rendering material semantics.

## 2.2 WASM voxelizer binding behavior

1. Main entrypoints currently construct mesh input with `material_ids: None`.
   - `crates/wasm_voxelizer/src/lib.rs:205`
   - `crates/wasm_voxelizer/src/lib.rs:743`
2. Options default to storing owner/color attributes (`store_owner`, `store_color`), reinforcing provenance/debug output path.
   - `crates/wasm_voxelizer/src/lib.rs:215`
   - `crates/wasm_voxelizer/src/lib.rs:754`

Conclusion: material IDs are not being carried from caller into voxelizer in the active WASM path.

## 2.3 TypeScript voxelizer/testbed path

1. Voxelizer request types expose positions/indices/grid/epsilon but no per-triangle material input.
   - `packages/voxelizer-js/src/index.ts:39`
2. Module output path emits a single voxel color tuple for visualization, not per-voxel materials.
   - `apps/web/src/modules/wasmVoxelizer/runCore.ts:158`
   - `apps/web/src/modules/wasmVoxelizer/runCore.ts:305`
3. `VoxelsDescriptor` carries a single optional `color` field.
   - `apps/web/src/modules/types.ts:10`

Conclusion: current UI/module path is visualization-oriented and materially non-authoritative.

## 2.4 Greedy mesher material capabilities (already present)

1. Core material type is `MaterialId = u16`.
   - `crates/greedy_mesher/src/core.rs:5`
2. Chunk storage already uses palette-based material compression.
   - `crates/greedy_mesher/src/chunk/palette_materials.rs:19`
3. Greedy merge logic is material-aware (`get_material` checks during quad merge).
   - `crates/greedy_mesher/src/merge/x_faces.rs:47`
4. Worker transfer path already includes `materialIds` in generated meshes.
   - `apps/web/src/modules/wasmGreedyMesher/workers/mesher.worker.ts:594`

Conclusion: greedy side is materially capable; voxelizer boundary is the missing link.

## 3. Requirements we must meet for materials

## 3.1 Correctness requirements

1. MAT-REQ-01: Deterministic material assignment per occupied voxel.
2. MAT-REQ-02: Deterministic tie-break policy when multiple triangles hit same voxel.
3. MAT-REQ-03: Consistent assignment across chunk boundaries and across runs.
4. MAT-REQ-04: Canonical empty material value maps to `MATERIAL_EMPTY = 0`.

## 3.2 Data contract requirements

1. MAT-REQ-05: Voxelizer external output must expose `u16 MaterialId`, not only owner/color.
2. MAT-REQ-06: Output must be chunk-native (`chunkCoord + local coords + material`) for direct chunk-manager ingestion.
3. MAT-REQ-07: Output format must avoid float-position authority for voxel edits.

## 3.3 Performance requirements

1. MAT-REQ-08: Preserve sparse-batch GPU memory safety characteristics.
2. MAT-REQ-09: Conversion overhead scales with occupied voxels, not full grid volume.
3. MAT-REQ-10: Minimize JS/WASM boundary chatter via chunk-grouped batch ingestion.
4. MAT-REQ-11: Avoid unnecessary palette churn (stable material IDs, deterministic mapping).

## 3.4 Pipeline integration requirements

1. MAT-REQ-12: Chunk manager receives bulk edits in chunk-grouped form.
2. MAT-REQ-13: Dirty marking, versioning, and neighbor sync remain owned by chunk manager.
3. MAT-REQ-14: Legacy sparse preview path remains optional debug mode only.

## 4. Architecture update plan (efficient materials support)

## 4.1 Principle

Do not rewrite voxelizer compute core. Move material semantics into the integration boundary and runtime contract.

## 4.2 Target material pipeline

1. Input stage:
   - Accept per-triangle material IDs at voxelizer WASM boundary.
2. Voxelization stage:
   - Continue sparse occupancy + owner selection (memory-safe).
3. Attribution stage:
   - Map winner `owner_id` -> canonical `MaterialId` using triangle-material table.
4. Conversion stage:
   - Convert sparse bricks to chunk-native deltas with `u16` materials.
5. Ingestion stage:
   - Apply chunk deltas in chunk manager with grouped raw writes + single version bump per chunk.
6. Meshing stage:
   - Existing material-aware merge and palette storage continue unchanged.

## 4.3 File-level architecture changes

## A. Voxelizer WASM input/output boundary

1. Add material-aware request path in `crates/wasm_voxelizer/src/lib.rs`.
2. Stop hardcoding `material_ids: None` for the new path.
3. Add new export returning chunk-delta payload with materials.

## B. Conversion module (Rust-side)

1. Add sparse->chunk conversion helper in `crates/wasm_voxelizer/src/`.
2. Emit:
   - `chunk_coord: [i32; 3]`
   - packed local coords (`0..61`)
   - `materials: Vec<u16>`
3. Keep dedupe and tie-break deterministic.

## C. Greedy WASM ingestion

1. Add `apply_chunk_deltas` in `crates/wasm_greedy_mesher/src/lib.rs`.
2. Route to chunk manager grouped raw writes in `crates/greedy_mesher/src/chunk/manager.rs`.

## D. Worker protocol and module runtime

1. Add chunk-delta message types in:
   - `apps/web/src/modules/wasmGreedyMesher/workers/chunkManagerTypes.ts`
2. Add handlers in:
   - `apps/web/src/modules/wasmGreedyMesher/workers/mesher.worker.ts`
   - `apps/web/src/modules/wasmGreedyMesher/workers/chunkManagerClient.ts`
3. Default runtime path switches from voxel positions to chunk deltas:
   - `apps/web/src/modules/wasmVoxelizer/runCore.ts`
   - `packages/voxelizer-js/src/index.ts`

## 4.4 Efficiency tactics

1. Keep sparse representation internal to voxelizer compute.
2. Perform conversion in Rust, not TypeScript.
3. Use chunk-grouped delta batches to reduce marshalling overhead.
4. Keep material IDs stable globally to reduce palette repacks.
5. Preserve legacy debug owner/color outputs only behind non-default flags.

## 5. Material mapping policy (must be explicit)

Before implementation lock, define:

1. Source of triangle material IDs (authoring/runtime provenance).
2. Owner-to-material mapping authority (direct table, not heuristic).
3. Tie-break behavior for overlapping triangles:
   - proposed default: lowest triangle index wins (consistent with current owner behavior).
4. Unknown material fallback:
   - map to `MATERIAL_DEFAULT` or fail fast in strict mode.

## 6. Validation and acceptance criteria

## 6.1 Functional checks

1. Material continuity across chunk boundaries.
2. No material loss from voxelizer output to meshed chunk output.
3. Deterministic results across repeated identical runs.

## 6.2 Performance checks

1. Conversion time budget stays proportional to occupied voxel count.
2. Reduced JS allocations vs legacy positions expansion path.
3. No regression in sparse dispatch stability on high-density scenes.

## 6.3 Observability checks

1. Track counts:
   - occupied voxels
   - emitted chunk edits
   - unique material IDs per chunk
2. Track chunk manager outcomes:
   - dirty chunks
   - rebuild/swap counts
   - version conflicts

## 7. Risks and mitigations

1. Risk: Ambiguous material authority leads to nondeterministic output.
   - Mitigation: explicit mapping table + deterministic tie-break contract.
2. Risk: Palette thrash from unstable IDs.
   - Mitigation: canonical material registry and stable IDs.
3. Risk: Boundary overhead erases gains.
   - Mitigation: chunk-grouped binary payloads, Rust-side conversion, phased rollout metrics.

## 8. Immediate next steps

1. Add material-specific requirement IDs from section 3 into:
   - `docs/greedy-meshing-docs/voxelizer-greedy-program-map.md`
2. Implement Phase 1 scaffolding:
   - chunk-delta types with `materials` payload in worker protocol.
3. Implement Phase 2:
   - material-aware chunk-delta export in `crates/wasm_voxelizer/src/lib.rs`.
4. Implement Phase 3:
   - `apply_chunk_deltas` ingestion in greedy WASM/chunk manager.

