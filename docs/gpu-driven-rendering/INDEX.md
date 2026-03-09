# GPU-Driven Rendering — Documentation Index

Date: March 9, 2026
Status: Authoritative

---

## What This Section Covers

This section documents the transition from Three.js-only rendering to a hybrid
GPU-driven pipeline for voxel chunk geometry. It defines why the transition is
necessary, what the target architecture looks like, and how to get there
incrementally.

This section does **not** cover the greedy meshing algorithm, chunk management,
voxelizer integration, or the radiance cascades algorithm. Those topics are
documented in their respective sections. This section covers the rendering
pipeline that consumes their outputs.

---

## Authority Hierarchy

When documents disagree, the following precedence applies:

| Priority | Document | Scope |
|----------|----------|-------|
| 1 | This section (all docs) | Rendering pipeline architecture |
| 2 | `docs/greedy-meshing-docs/` ADRs | Meshing, chunks, materials |
| 3 | `docs/culling/` | Hi-Z culling details |
| 4 | `docs/greedy-meshing-docs/threejs-buffer-management.md` | **Superseded** for chunk rendering by this section (remains valid for non-chunk Three.js objects) |

---

## Reading Order by Persona

### New to this topic
1. [`../architecture-map.md`](../architecture-map.md) — **master map** of all data structures and their relationships
2. [`philosophy.md`](philosophy.md) — why three features converge on the same architectural gap
3. [`adr/0011-hybrid-gpu-driven.md`](adr/0011-hybrid-gpu-driven.md) — the core decision
4. [`design/three-js-limits.md`](design/three-js-limits.md) — concrete evidence

### Implementing the pipeline
1. [`design/pipeline-architecture.md`](design/pipeline-architecture.md) — target frame pipeline and timing
2. [`spec/frame-graph.md`](spec/frame-graph.md) — pass ordering and resource dependencies
3. [`design/hybrid-transition.md`](design/hybrid-transition.md) — phased migration plan

### Working on culling
1. [`spec/visibility-buffer.md`](spec/visibility-buffer.md) — meshlet/cluster design
2. [`../culling/hiz-occlusion-culling-report.md`](../culling/hiz-occlusion-culling-report.md) — Hi-Z depth pyramid details

### Working on radiance cascades
1. [`design/pipeline-architecture.md`](design/pipeline-architecture.md) — where cascades fit in the frame
2. [`spec/frame-graph.md`](spec/frame-graph.md) — Pass 4 dependencies
3. [`../greedy-meshing-docs/adr/0010-radiance-cascades.md`](../greedy-meshing-docs/adr/0010-radiance-cascades.md) — cascade algorithm details

### LLM context window load (minimal set)
1. This file (`INDEX.md`)
2. [`philosophy.md`](philosophy.md)
3. [`design/pipeline-architecture.md`](design/pipeline-architecture.md)
4. [`spec/frame-graph.md`](spec/frame-graph.md)

---

## Document Map

### Philosophy and Decision

| File | Topic | Status |
|------|-------|--------|
| [`philosophy.md`](philosophy.md) | The convergence argument; why Three.js is insufficient | Authoritative |
| [`adr/0011-hybrid-gpu-driven.md`](adr/0011-hybrid-gpu-driven.md) | Formal decision; amends ADR-0001 | Proposed |

### Design (What Must Be True)

| File | Topic | Status |
|------|-------|--------|
| [`design/three-js-limits.md`](design/three-js-limits.md) | Specific Three.js limitations with code evidence | Authoritative |
| [`design/pipeline-architecture.md`](design/pipeline-architecture.md) | Target frame pipeline, resource table, timing budgets | Authoritative |
| [`design/hybrid-transition.md`](design/hybrid-transition.md) | 5-phase incremental migration with rollback strategy | Authoritative |

### Specification (How It Works)

| File | Topic | Status |
|------|-------|--------|
| [`spec/frame-graph.md`](spec/frame-graph.md) | Pass definitions, resource dependency graph, synchronization | Authoritative |
| [`spec/visibility-buffer.md`](spec/visibility-buffer.md) | Meshlet/cluster culling; visibility buffer rendering | Future |

---

## Relationship to Other Documentation

```
docs/
├── gpu-driven-rendering/     ← THIS SECTION (rendering pipeline)
│   ├── Consumes output from:
│   │   ├── greedy-meshing-docs/   (chunk mesh data, material system)
│   │   └── voxelizer-integration/ (occupancy data for cascades)
│   │
│   ├── Implements features from:
│   │   ├── greedy-meshing-docs/adr/0010-radiance-cascades.md
│   │   └── culling/hiz-occlusion-culling-report.md
│   │
│   └── Amends:
│       └── greedy-meshing-docs/adr/0001-renderer-choice.md
│
├── greedy-meshing-docs/      (meshing algorithm, chunk system, materials)
├── voxelizer-integration/    (GPU voxelizer → chunk manager flow)
└── culling/                  (Hi-Z research and gap analysis)
```

---

## Key ADRs Across Sections

| ADR | Section | Decision | Relevance |
|-----|---------|----------|-----------|
| [0001](../greedy-meshing-docs/adr/0001-renderer-choice.md) | Greedy meshing | Three.js renderer | **Amended** by ADR-0011 |
| [0003](../greedy-meshing-docs/adr/0003-binary-greedy-meshing.md) | Greedy meshing | Binary greedy meshing | Provides mesh data consumed by pipeline |
| [0007](../greedy-meshing-docs/adr/0007-material-strategy.md) | Greedy meshing | Material atlas + UVs | Fragment shader reads atlas |
| [0009](../voxelizer-integration/adr/0009-architecture-b.md) | Voxelizer | GPU-compact integration | Provides occupancy data for cascades |
| [0010](../greedy-meshing-docs/adr/0010-radiance-cascades.md) | Greedy meshing | Radiance cascades | Pass 4 of the frame pipeline |
| [0011](adr/0011-hybrid-gpu-driven.md) | **This section** | Hybrid GPU-driven pipeline | Core rendering decision |

---

## Implementation Phases (Summary)

| Phase | Description | Key Deliverable |
|-------|-------------|-----------------|
| **0** | Shared GPU device | `sharedDevice` on ViewerBackend |
| **1** | Depth prepass | App-owned depth texture readable in compute |
| **2** | Compute infrastructure | Hi-Z pyramid + radiance cascade compute |
| **3** | Custom color pass | Chunk meshes render through custom pipeline with GI |
| **4** | Indirect draw + GPU culling | GPU decides visibility, single indirect draw |
| **5** | Fine-grained culling | Per-cluster AABBs, backface rejection |

See [`design/hybrid-transition.md`](design/hybrid-transition.md) for detailed phase specifications.
