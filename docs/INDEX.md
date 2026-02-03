# Voxel Mesh System - Documentation Index

Quick navigation for the voxel-to-mesh rendering architecture.

---

## Document Map

```
                              ┌─────────────────────────┐
                              │   voxel-mesh-           │
                              │   architecture.md       │  ← START HERE
                              │   (Overview)            │
                              └───────────┬─────────────┘
                                          │
          ┌───────────────────────────────┼───────────────────────────────┐
          │                               │                               │
          ▼                               ▼                               ▼
┌─────────────────────┐      ┌─────────────────────┐      ┌─────────────────────┐
│ implementation-     │      │ greedy-mesh-        │      │ architecture-       │
│ plan.md             │      │ implementation-     │      │ addendum.md         │
│ (Phases/Milestones) │      │ plan.md (Algorithm) │      │ (Gap Solutions)     │
└─────────────────────┘      └──────────┬──────────┘      └─────────────────────┘
                                        │
                                        ▼
                             ┌─────────────────────┐
                             │ binary-greedy-      │
                             │ meshing-analysis.md │
                             │ (10-50x Speedup)    │
                             └─────────────────────┘

┌─────────────────────┐      ┌─────────────────────┐      ┌─────────────────────┐
│ chunk-management-   │      │ threejs-buffer-     │      │ development-        │
│ system.md           │      │ management.md       │      │ guidelines.md       │
│ (State Machine)     │      │ (GPU Buffers)       │      │ (Coding Standards)  │
└─────────────────────┘      └─────────────────────┘      └─────────────────────┘
```

---

## All Documents

| Document | Purpose | Key Sections |
|----------|---------|--------------|
| [voxel-mesh-architecture.md](voxel-mesh-architecture.md) | High-level overview, requirements | Architecture diagram, REQ-* tables, Data flow |
| [implementation-plan.md](implementation-plan.md) | Phased milestones, deliverables | Phase 1-6, **Testing Strategy**, Debug tooling, Benchmarks |
| [greedy-mesh-implementation-plan.md](greedy-mesh-implementation-plan.md) | Binary meshing algorithm (Rust) | Data structures, Bitwise culling, Greedy merge |
| [binary-greedy-meshing-analysis.md](binary-greedy-meshing-analysis.md) | Reference algorithm analysis | Bitmask representation, Performance analysis |
| [architecture-addendum.md](architecture-addendum.md) | Gap solutions, cross-cutting concerns | WASM safety, **Cross-language logging**, Material strategy |
| [chunk-management-system.md](chunk-management-system.md) | Dirty tracking, state machine | ChunkState enum, RebuildQueue, Version consistency |
| [threejs-buffer-management.md](threejs-buffer-management.md) | GPU buffer lifecycle | ChunkMeshPool, Double-buffering, Clipping planes |
| [development-guidelines.md](development-guidelines.md) | Coding standards | File limits, Function design, Error handling |
| [typescript-architecture.md](typescript-architecture.md) | TypeScript patterns and debugging | Branded types, State machines, Logger, Inspector |
| [greedy-mesher-crate-structure.md](greedy-mesher-crate-structure.md) | Rust crate organization | Module structure, WASM bindings, Build integration |
| [instanced-mesh-chunking-debug-report.md](instanced-mesh-chunking-debug-report.md) | Historical: WebGPU bug investigation | Debug attempts (led to greedy mesh pivot) |

### Architecture Decision Records (ADRs)

| ADR | Decision | Status |
|-----|----------|--------|
| [ADR-0001](adr/0001-renderer-choice.md) | Renderer choice | Accepted |
| [ADR-0002](adr/0002-module-contract.md) | Module contract | Accepted |
| [ADR-0003](adr/0003-binary-greedy-meshing.md) | Binary greedy meshing algorithm | Accepted |
| [ADR-0004](adr/0004-chunk-size-64.md) | 64³ chunk size | Accepted |
| [ADR-0005](adr/0005-voxelizer-to-mesher-integration.md) | Voxelizer to mesher integration | Proposed |
| [ADR-0006](adr/0006-lod-strategy.md) | Level of Detail (LOD) strategy | Proposed |
| [ADR-0007](adr/0007-material-strategy.md) | Material strategy (textures, per-voxel) | Proposed |
| [ADR-0008](adr/0008-design-gap-mitigations.md) | Design gap mitigations (12 issues) | Proposed |

---

## Reading Order

### New to the project?
1. [voxel-mesh-architecture.md](voxel-mesh-architecture.md) - Understand the big picture
2. [implementation-plan.md](implementation-plan.md) - See what's being built and when
3. [development-guidelines.md](development-guidelines.md) - Coding standards before contributing

### Implementing the mesher?
1. [greedy-mesh-implementation-plan.md](greedy-mesh-implementation-plan.md) - Full Rust implementation
2. [binary-greedy-meshing-analysis.md](binary-greedy-meshing-analysis.md) - Why binary approach is 10-50x faster
3. [architecture-addendum.md](architecture-addendum.md) - Migration path and WASM API

### Working on chunk system?
1. [chunk-management-system.md](chunk-management-system.md) - State machine and dirty tracking
2. [architecture-addendum.md](architecture-addendum.md) - Cross-chunk boundary handling

### Working on rendering?
1. [threejs-buffer-management.md](threejs-buffer-management.md) - Buffer lifecycle
2. [architecture-addendum.md](architecture-addendum.md) - Output format and Three.js integration

### Writing tests or debugging?
1. [implementation-plan.md](implementation-plan.md#testing-strategy) - Test frameworks, per-phase tests, CI workflow
2. [architecture-addendum.md](architecture-addendum.md#9-cross-language-logging) - Unified Rust/TS logging, perf tracing
3. [implementation-plan.md](implementation-plan.md#performance-benchmarks) - Benchmark thresholds and specs

### Integrating voxelizer with mesher?
1. [ADR-0005](adr/0005-voxelizer-to-mesher-integration.md) - **Start here**: Gap analysis and integration plan
2. [architecture-addendum.md](architecture-addendum.md#2-voxelizer--chunk-conversion) - Conversion approach
3. [greedy-mesh-implementation-plan.md](greedy-mesh-implementation-plan.md#part-2-input-conversion) - Input format requirements

### Working on TypeScript layer?
1. [typescript-architecture.md](typescript-architecture.md) - **Start here**: Type patterns, state machines, debugging
2. [development-guidelines.md](development-guidelines.md) - General coding standards
3. [chunk-management-system.md](chunk-management-system.md) - State machine design (Rust reference)

### Working on materials/textures?
1. [ADR-0007](adr/0007-material-strategy.md) - Material strategy decision
2. [greedy-mesh-implementation-plan.md](greedy-mesh-implementation-plan.md) - UV generation context
3. [typescript-architecture.md](typescript-architecture.md#resource-lifecycle) - Three.js resource management

### Reviewing design robustness?
1. [ADR-0008](adr/0008-design-gap-mitigations.md) - **Start here**: All 12 gap mitigations
2. [chunk-management-system.md](chunk-management-system.md#8-backpressure-strategy) - Backpressure, snapshots
3. [threejs-buffer-management.md](threejs-buffer-management.md#9-true-preallocation-with-tiered-pools) - Corrected preallocation

---

## Quick Reference

### Requirements Index

All requirements are in [voxel-mesh-architecture.md](voxel-mesh-architecture.md):

| Prefix | Category | Count |
|--------|----------|-------|
| REQ-IN-* | Input voxel field | 5 |
| REQ-SURF-* | Surface extraction | 4 |
| REQ-GEO-* | Geometry output | 5 |
| REQ-CHUNK-* | Chunk management | 6 |
| REQ-RENDER-* | Rendering | 5 |
| REQ-PERF-* | Performance | 4 |
| REQ-DEBUG-* | Debugging | 6 |

### Key Constants

| Constant | Value | Location |
|----------|-------|----------|
| `CHUNK_SIZE` | 64 | [greedy-mesh-implementation-plan.md](greedy-mesh-implementation-plan.md#part-1-data-structures) |
| `CHUNK_SIZE_USABLE` | 62 | (1-voxel padding on each side) |
| `CS_P` | 64 | Chunk Size with Padding |
| `CS` | 62 | Usable Chunk Size |
| Packed quad size | 8 bytes | [binary-greedy-meshing-analysis.md](binary-greedy-meshing-analysis.md#12-quad-encoding) |

### Key Types

**Documented (Mesher):**

| Type | Purpose | Location |
|------|---------|----------|
| `BinaryChunk` | Bitmask + types storage | [greedy-mesh-implementation-plan.md](greedy-mesh-implementation-plan.md#binary-chunk-representation) |
| `FaceMasks` | Visible face bits per direction | [greedy-mesh-implementation-plan.md](greedy-mesh-implementation-plan.md#face-masks-storage) |
| `ChunkState` | Clean/Dirty/Meshing/ReadyToSwap | [chunk-management-system.md](chunk-management-system.md) |
| `MeshOutput` | positions/normals/indices | [greedy-mesh-implementation-plan.md](greedy-mesh-implementation-plan.md#mesh-output) |
| `VoxelMeshDescriptor` | TypeScript output type | [architecture-addendum.md](architecture-addendum.md#step-1-extend-type-definitions) |

**Implemented (Voxelizer):**

| Type | Purpose | Location |
|------|---------|----------|
| `SparseVoxelizationOutput` | Sparse brick voxel data | `crates/voxelizer/src/core.rs` |
| `VoxelGridSpec` | Grid origin, size, dimensions | `crates/voxelizer/src/core.rs` |
| `GpuVoxelizer` | WebGPU voxelization engine | `crates/voxelizer/src/gpu.rs` |

> ⚠️ **Note:** These two type systems are incompatible. See [ADR-0005](adr/0005-voxelizer-to-mesher-integration.md) for integration plan.

### Performance Targets

| Metric | Target | Source |
|--------|--------|--------|
| Chunk mesh time | <200 µs | [implementation-plan.md](implementation-plan.md) |
| Typical terrain chunk | ~74 µs | [binary-greedy-meshing-analysis.md](binary-greedy-meshing-analysis.md) |
| Frame budget | N chunks/frame (configurable) | [chunk-management-system.md](chunk-management-system.md) |
| Memory per chunk | ~32 KB opaque mask + 256 KB types | [voxel-mesh-architecture.md](voxel-mesh-architecture.md#memory-layout) |

---

## Implementation Phases

| Phase | Status | Key Deliverables |
|-------|--------|------------------|
| **Phase 1: Core Meshing** | Not started | BinaryChunk, cull_faces(), greedy merge, WASM bindings |
| **Phase 2: Chunk System** | Not started | ChunkManager, DirtyTracker, RebuildQueue |
| **Phase 3: Render Integration** | Not started | ChunkMeshPool, double-buffering, clipping planes |
| **Phase 4: Debug Tooling** | Not started | Quad visualization, chunk boundaries, state overlay |
| **Phase 5: Optimization** | Not started | Web Workers, LOD, memory budget |
| **Phase 6: Polish** | Not started | Error handling, edge cases, docs |

See [implementation-plan.md](implementation-plan.md) for detailed milestones and checklists.

---

## File Locations (Implementation)

When implementation begins, new files will be added:

```
crates/
├── greedy_mesher/            # NEW: Core meshing library
│   └── src/
│       ├── lib.rs            # Public exports
│       ├── core.rs           # BinaryChunk, FaceMasks, MeshOutput
│       ├── convert.rs        # Input conversion
│       ├── cull.rs           # Bitwise face culling
│       ├── merge/            # Greedy merge per axis
│       ├── expand.rs         # Quad expansion to vertices
│       └── mesh.rs           # Pipeline
│
└── wasm_greedy_mesher/       # NEW: WASM bindings
    └── src/lib.rs            # mesh_voxel_positions(), mesh_dense_voxels()

apps/web/src/
├── voxel/                    # NEW: TypeScript voxel subsystem
│   ├── types.ts              # Branded types, state unions
│   ├── chunk/                # ChunkManager, DirtyTracker, etc.
│   ├── mesh/                 # ChunkMeshPool, MaterialManager
│   ├── edit/                 # VoxelEditor, EditHistory
│   ├── wasm/                 # WasmBridge, WorkerPool
│   └── debug/                # Inspector, Logger, PerformanceTracer
│
└── wasm/
    └── wasm_greedy_mesher/   # Generated by wasm-pack

modules/types.ts              # MODIFY: Add VoxelMeshDescriptor
viewer/outputs.ts             # MODIFY: Add buildVoxelMesh()
```

See [greedy-mesher-crate-structure.md](greedy-mesher-crate-structure.md) for detailed crate organization.

---

## Change Log

| Date | Change |
|------|--------|
| 2026-02-03 | Created ADR-0008 documenting 12 design gap mitigations (memory, backpressure, snapshots, etc.) |
| 2026-02-03 | Updated chunk-management-system.md with backpressure, snapshots, sparse storage, neighbor policy |
| 2026-02-03 | Updated threejs-buffer-management.md with true preallocation (tiered pools) |
| 2026-02-03 | Updated architecture-addendum.md with robust coordinate conversion (epsilon tolerance) |
| 2026-02-03 | Updated implementation-plan.md with worker-ready API design note |
| 2026-02-03 | Created ADR-0007 documenting material strategy (textures, per-voxel materials) |
| 2026-02-03 | Created typescript-architecture.md (branded types, state machines, debugging) |
| 2026-02-03 | Created greedy-mesher-crate-structure.md (Rust crate organization) |
| 2026-01-29 | Created ADR-0006 documenting LOD strategy options (recommends point mode) |
| 2026-01-29 | Created ADR-0005 documenting voxelizer-to-mesher integration gap |
| 2026-01-29 | Added comprehensive Testing Strategy (per-phase tests, CI workflow) |
| 2026-01-29 | Added Cross-Language Logging specification (Rust + TypeScript) |
| 2026-01-29 | Added WASM Memory Safety and WebGPU 64-bit limitation docs |
| 2026-01-29 | Created ADR-0003 (binary greedy meshing) and ADR-0004 (64³ chunks) |
| 2026-01-28 | Adopted binary greedy meshing (10-50x speedup) |
| 2026-01-28 | Changed chunk size from 32³ to 64³ |
| 2026-01-28 | Created comprehensive documentation set |
