> **SUPERSEDED** — This document's authority hierarchy and navigation links are outdated.
> The current authoritative navigation hub is `docs/voxelizer-integration/INDEX.md`.
> This file is retained as a historical record only.

---

# Voxelizer + Greedy Program Map

Date: February 20, 2026
Status: Active navigation + traceability hub
Audience: Engineers implementing voxelizer->greedy integration and related rendering systems

## 1. Why this file exists

The project has multiple documents that cover adjacent parts of the same architecture effort. This map defines:

1. What each document is for.
2. Which document is authoritative for which decision type.
3. How to move through docs during implementation day without context thrash.

## 2. Source-of-truth hierarchy

Use this precedence when guidance conflicts:

1. Prescriptive migration spec:
   `docs/greedy-meshing-docs/voxelizer-greedy-native-migration-outline.md`
2. Architecture decisions:
   `docs/greedy-meshing-docs/adr/0005-voxelizer-to-mesher-integration.md`
3. Descriptive system framing:
   `docs/greedy-meshing-docs/voxelizer-greedy-mesher-unification-report.md`
4. Historical rationale:
   `docs/greedy-meshing-docs/original-reasoning-sparse-brick-occupancy-batches.md`
5. Rendering adjunct (occlusion path):
   `docs/culling/hiz-occlusion-culling-report.md`

## 3. Document roles and relationships

| Document | Type | Purpose | Inputs | Outputs |
|----------|------|---------|--------|---------|
| [voxelizer-greedy-mesher-unification-report.md](voxelizer-greedy-mesher-unification-report.md) | Descriptive | Defines integration problem space and mismatch domains | Current code reality | Problem statements and unresolved questions |
| [original-reasoning-sparse-brick-occupancy-batches.md](original-reasoning-sparse-brick-occupancy-batches.md) | Descriptive/Historical | Explains why sparse batching exists and what constraints it solved | GPU/runtime limits and historical failures | Legacy rationale and non-regression constraints |
| [voxelizer-greedy-native-migration-outline.md](voxelizer-greedy-native-migration-outline.md) | Prescriptive | Defines target contract, migration phases, and implementation tasks | Unification report + historical rationale | Build plan, contract shape, implementation sequence |
| [voxelizer-materials-state-requirements-architecture-report.md](voxelizer-materials-state-requirements-architecture-report.md) | Prescriptive deep-dive | Defines materials requirements and architecture updates for voxelizer->greedy path | Migration outline + current material code reality | MAT-REQ contract and implementation priorities |
| [hiz-occlusion-culling-report.md](../culling/hiz-occlusion-culling-report.md) | Prescriptive adjunct | Defines Hi-Z readiness and required render/culling data contracts | Current viewer/backend + greedy target | Culling implementation constraints and staged path |

## 4. Implementation-day reading order

Use this sequence before and during coding:

1. Read problem framing:
   `docs/greedy-meshing-docs/voxelizer-greedy-mesher-unification-report.md`
2. Read sparse legacy constraints (to avoid regressing hard-won behavior):
   `docs/greedy-meshing-docs/original-reasoning-sparse-brick-occupancy-batches.md`
3. Execute against the migration contract:
   `docs/greedy-meshing-docs/voxelizer-greedy-native-migration-outline.md`
4. If touching material semantics, validate against:
   `docs/greedy-meshing-docs/voxelizer-materials-state-requirements-architecture-report.md`
5. If rendering/culling code is touched, validate against:
   `docs/culling/hiz-occlusion-culling-report.md`

## 5. Workstream checkpoints

## 5.1 Contract checkpoint

Before coding:

1. Confirm output contract fields and ownership in migration outline section 5.
2. Confirm chunk coordinate semantics (`CS=62`) in migration outline section 3.

## 5.2 Conversion checkpoint

While implementing voxelizer export:

1. Validate brick->chunk conversion rules from migration outline section 6.
2. Validate sparse constraints from historical report sections 3-7.

## 5.3 Runtime ingestion checkpoint

While wiring worker/chunk manager:

1. Align with migration outline section 7 and section 8 phases 3-4.
2. Keep chunk manager state/update semantics as primary authority.

## 5.4 Rendering checkpoint

When integrating visibility or culling:

1. Validate with Hi-Z report sections 4-9.
2. Keep cull units aligned with greedy chunk ownership, not preview voxel instances.

## 6. Traceability matrix (requirements -> docs -> code)

| Requirement | Primary doc section | Primary code touchpoints |
|-------------|---------------------|--------------------------|
| VG-REQ-01 Chunk-native voxelizer output | Migration outline section 5 | `crates/wasm_voxelizer/src/lib.rs`, `apps/web/src/modules/wasmGreedyMesher/workers/chunkManagerTypes.ts` |
| VG-REQ-02 Preserve sparse memory-safe behavior | Historical report sections 3-7 | `crates/voxelizer/src/gpu/mod.rs`, `crates/voxelizer/src/gpu/sparse.rs` |
| VG-REQ-03 Deterministic material flow to `u16` | Unification section 4.2 + migration section 5 | `crates/wasm_voxelizer/src/lib.rs`, `crates/greedy_mesher/src/core.rs` |
| VG-REQ-04 Chunk-manager-native ingestion path | Migration section 7 + section 8 phase 3 | `crates/wasm_greedy_mesher/src/lib.rs`, `crates/greedy_mesher/src/chunk/manager.rs` |
| VG-REQ-05 Greedy-chunk-aligned culling metadata | Hi-Z report sections 6-9 | `apps/web/src/viewer/threeBackend.ts`, `apps/web/src/modules/types.ts` |
| MAT-REQ-01..14 Materials requirements | Materials report sections 3-4 | `crates/wasm_voxelizer/src/lib.rs`, `crates/wasm_greedy_mesher/src/lib.rs`, `apps/web/src/modules/wasmGreedyMesher/workers/chunkManagerTypes.ts` |

## 7. Doc maintenance protocol

When implementation changes scope:

1. Update `voxelizer-greedy-native-migration-outline.md` first.
2. If the change alters assumptions, update `voxelizer-greedy-mesher-unification-report.md`.
3. If the change affects sparse legacy constraints, update `original-reasoning-sparse-brick-occupancy-batches.md`.
4. If material semantics or material transport changes, update `voxelizer-materials-state-requirements-architecture-report.md`.
5. If render-pass or culling contracts change, update `hiz-occlusion-culling-report.md`.
6. Keep this program map updated with any new canonical docs.

## 8. Recommended next step

Begin implementation at migration outline phase 1 and use this map as the checklist spine during code review and PR validation.
