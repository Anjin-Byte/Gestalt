# Superseded Documents

Date: February 22, 2026

---

## What This Directory Contains

Documents from an earlier integration design phase (Architecture A) that have been
superseded by the organized `docs/voxelizer-integration/` section.

**Do not implement from these documents.** They describe a CPU-side conversion
approach (occupancy bit scanning, CPU material lookup, intermediate wire format)
that was replaced by Architecture B (GPU compact pass with material resolution).

**The valuable analysis in these documents has been preserved** — not discarded —
into the new section. See the table below for where each document's content lives.

---

## Supersession Table

| Document | Why Superseded | Content Preserved In |
|----------|---------------|----------------------|
| `voxelizer-chunk-native-output-design-requirements.md` | Reorganized and expanded into the full section | `design/requirements.md`, `design/gpu-output-contract.md`, `design/cpu-ingestion.md` |
| `voxelizer-greedy-integration-spec.md` | Architecture A impl plan; coord frames and material pipeline content are Architecture B-compatible and preserved | `spec/coordinate-frames.md`, `spec/material-pipeline.md`, `spec/wasm-api.md`, `spec/invariants.md` |
| `voxelizer-greedy-integration-implementation-plan.md` | Architecture A (CPU occupancy scan); superseded by `impl/` docs | `impl/overview.md`, `impl/greedy-voxelizer-crate.md`, `impl/wasm-bindings.md` |
| `voxelizer-greedy-native-migration-outline.md` | Architecture A; project goals preserved in requirements | `design/requirements.md`, `adr/0009-architecture-b.md` |
| `voxelizer-materials-state-requirements-architecture-report.md` | Architecture A impl plan; MAT-REQ requirements preserved | `design/requirements.md`, `spec/material-pipeline.md` |
| `voxelizer-greedy-program-map.md` | Authority hierarchy outdated; superseded by `INDEX.md` | `INDEX.md` |
| `voxelizer-greedy-mesher-unification-report.md` | Historical problem framing; insight preserved in `philosophy.md` | `philosophy.md` |
| `original-reasoning-sparse-brick-occupancy-batches.md` | Historical GPU design rationale | `philosophy.md` (referenced) |

---

## Key Difference: Architecture A vs Architecture B

**Architecture A (these docs):**
- GPU outputs `SparseVoxelizationOutput` (brick-based, dense, with occupancy bits)
- CPU iterates occupancy bits to find occupied voxels (O(total_grid_voxels))
- CPU looks up `material_table[owner_id]` per voxel
- CPU reconstructs coordinates from `brick_origin + local_xyz`
- CPU sends intermediate wire format to chunk manager

**Architecture B (current design):**
- GPU compact pass resolves materials and outputs only occupied voxels
- GPU outputs `CompactVoxel[] {vx, vy, vz, material}` with global i32 coords
- CPU groups by `div_euclid(CS=62)` chunk coordinate only
- No CPU occupancy scan. No CPU material lookup. No intermediate format.

The canonical store principle drives this choice. See `../philosophy.md`.
