# Voxelizer Integration — Documentation Index

Date: February 22, 2026
Status: Authoritative

---

## What This Section Covers

This section documents the integration of the GPU voxelizer (`crates/voxelizer`)
with the greedy mesh chunk manager (`crates/greedy_mesher`). It defines the
design, specification, and implementation plan for a single-call API that
voxelizes a triangle mesh, resolves per-voxel materials, and writes the result
directly into the chunk manager's canonical voxel store.

This section does **not** cover the greedy meshing algorithm itself, chunk
management lifecycle, Three.js buffer management, or LOD strategy. Those topics
are documented in [`docs/greedy-meshing-docs/`](../greedy-meshing-docs/INDEX.md).

---

## Authority Hierarchy

When documents disagree, the following precedence applies:

| Priority | Document | Scope |
|----------|----------|-------|
| 1 | This section (all docs) | Voxelizer integration — all topics |
| 2 | `docs/greedy-meshing-docs/` ADRs 0001-0004, 0006-0008 | Greedy meshing system |
| 3 | `archive/` docs | Historical context only — do not implement from these |

Within this section, `design/` documents define requirements (what must be true).
`spec/` documents define specifications (how it works). `impl/` documents define
the implementation plan (what to build). If a `spec/` doc and a `design/` doc
appear to conflict, the `design/` doc takes precedence.

---

## Reading Order by Persona

### New to this integration
1. `philosophy.md` — the architectural principle driving all decisions
2. `adr/0009-architecture-b.md` — the key design decision and its rationale
3. `design/requirements.md` — what the system must satisfy

### Implementing the GPU changes
1. `spec/coordinate-frames.md` — understand the three coordinate spaces
2. `design/gpu-output-contract.md` — what the GPU must produce
3. `impl/gpu-shader-changes.md` — exact shader modifications

### Implementing the CPU ingestion
1. `design/cpu-ingestion.md` — what the CPU does with GPU output
2. `spec/invariants.md` — correctness constraints to enforce
3. `impl/greedy-voxelizer-crate.md` — new crate specification

### Implementing the WASM API
1. `spec/wasm-api.md` — API signatures and worker protocol
2. `impl/wasm-bindings.md` — WasmChunkManager extension

### LLM context window load (minimal set)
Load in this order for maximum coverage per token:
1. This file (`INDEX.md`)
2. `philosophy.md`
3. `design/gpu-output-contract.md`
4. `design/cpu-ingestion.md`
5. `spec/invariants.md`

### Full audit / review
Load all `design/`, `spec/`, and `impl/` files. Ignore `archive/`.

---

## Document Map

### Philosophy and Decision

| File | Topic | Status |
|------|-------|--------|
| `philosophy.md` | Canonical store principle; why Architecture B | Authoritative |
| `adr/0009-architecture-b.md` | Formal decision record; supersedes ADR-0005 | Accepted |

### Design (What Must Be True)

| File | Topic | Status |
|------|-------|--------|
| `design/requirements.md` | Integration and material requirements | Authoritative |
| `design/gpu-output-contract.md` | GPU compact pass output guarantees | Authoritative |
| `design/cpu-ingestion.md` | CPU chunk grouping and palette write contract | Authoritative |

### Specification (How It Works)

| File | Topic | Status |
|------|-------|--------|
| `spec/coordinate-frames.md` | Three coordinate spaces; VOX-ALIGN; VOX-SIZE | Authoritative |
| `spec/material-pipeline.md` | material_table → MaterialId flow; tie-break policy | Authoritative |
| `spec/wasm-api.md` | WASM API signatures; worker protocol | Authoritative |
| `spec/invariants.md` | All formal correctness invariants | Authoritative |

### Implementation (What to Build)

| File | Topic | Status |
|------|-------|--------|
| `impl/overview.md` | File-by-file change map; build order | Authoritative |
| `impl/gpu-shader-changes.md` | COMPACT_ATTRS_WGSL modifications | Authoritative |
| `impl/greedy-voxelizer-crate.md` | New `crates/greedy_voxelizer` specification | Authoritative |
| `impl/wasm-bindings.md` | WasmChunkManager extension | Authoritative |

### Archive (Superseded — do not implement from these)

| File | Superseded by |
|------|---------------|
| `archive/voxelizer-chunk-native-output-design-requirements.md` | This section |
| `archive/voxelizer-greedy-integration-spec.md` | `spec/` + `impl/` docs |
| `archive/voxelizer-greedy-integration-implementation-plan.md` | `impl/` docs |
| `archive/voxelizer-greedy-native-migration-outline.md` | `adr/0009`, `design/requirements.md` |
| `archive/voxelizer-materials-state-requirements-architecture-report.md` | `design/requirements.md`, `spec/material-pipeline.md` |
| `archive/voxelizer-greedy-program-map.md` | This `INDEX.md` |
| `archive/voxelizer-greedy-mesher-unification-report.md` | `philosophy.md` |
| `archive/original-reasoning-sparse-brick-occupancy-batches.md` | `philosophy.md` |

See `archive/SUPERSEDED.md` for migration notes.

---

## Quick Reference

### Key Constants

| Constant | Value | Source |
|----------|-------|--------|
| `CS` | `62` | Usable voxels per chunk side — `crates/greedy_mesher/src/core.rs:16` |
| `CS_P` | `64` | Padded chunk side (CS + 2 border voxels) |
| `MATERIAL_EMPTY` | `0` | Air/unoccupied — never written to a solid voxel |
| `MATERIAL_DEFAULT` | `1` | Solid with no explicit material |
| GPU sentinel | `0xFFFFFFFF` | Unresolved owner in compact output |

### Invariant Names

| Name | Short description | Full spec |
|------|-------------------|-----------|
| VOX-ALIGN | `origin_world[i]` must be a multiple of `voxel_size` | `spec/coordinate-frames.md` |
| VOX-SIZE | `voxel_size` must equal `manager.voxel_size()` | `spec/coordinate-frames.md` |
| VOX-PARTIAL | GPU writes only to voxels it has data for; no clearing of out-of-grid slots | `design/cpu-ingestion.md` |
| C1 | Occupancy conservation | `spec/invariants.md` |
| C2 | Local coordinate range `[0, 62)` | `spec/invariants.md` |
| C3 | Material validity (no MATERIAL_EMPTY to solid voxel) | `spec/invariants.md` |
| C4 | Chunk coordinate round-trip consistency | `spec/invariants.md` |

### Key Source Locations

| Symbol | File | Line |
|--------|------|------|
| `CS = 62` | `crates/greedy_mesher/src/core.rs` | 16 |
| `MaterialId = u16` | `crates/greedy_mesher/src/core.rs` | 5 |
| `MATERIAL_EMPTY`, `MATERIAL_DEFAULT` | `crates/greedy_mesher/src/core.rs` | 8–10 |
| `VoxelGridSpec` | `crates/voxelizer/src/core.rs` | 4 |
| `SparseVoxelizationOutput` | `crates/voxelizer/src/core.rs` | 143 |
| `ChunkCoord::from_voxel` (`div_euclid`) | `crates/greedy_mesher/src/chunk/coord.rs` | 82–89 |
| `Chunk::set_voxel_raw` | `crates/greedy_mesher/src/chunk/chunk.rs` | 149–153 |
| `ChunkManager::set_voxels_batch` | `crates/greedy_mesher/src/chunk/manager.rs` | 214–248 |
| `COMPACT_ATTRS_WGSL` shader | `crates/voxelizer/src/gpu/shaders.rs` | (search string) |
| `compact_sparse_attributes` | `crates/voxelizer/src/gpu/compact_attrs.rs` | (search string) |
| `WasmChunkManager` | `crates/wasm_greedy_mesher/src/lib.rs` | (search string) |
