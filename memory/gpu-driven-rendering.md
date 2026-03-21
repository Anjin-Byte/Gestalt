---
name: GPU-Driven Rendering Decisions
description: ADR-0010/0011 decisions plus Resident Representation three-layer model and authoritative pipeline spec
type: project
---

## Authoritative Spec

**`docs/Resident Representation/`** (symlinked at `memory/Resident Representation/`) is the current authoritative architecture. All documents marked Done. ADRs 0010/0011 predate it and are consistent with it but less detailed. When in conflict, the Resident Representation wins.

Key files:
- `pipeline-stages.md` — exact buffer/texture ownership per stage (supersedes the rough pipeline below)
- `layer-model.md` — three-product model (see below)
- `chunk-contract.md` — canonical chunk fields, authoritativeness rules
- `radiance-cascades-impl.md` — cascade impl grounded in GPU-resident chunks
- `depth-prepass.md` — depth prepass + Hi-Z raster optimization chain
- `meshlets.md` — sub-chunk visibility via meshlet clusters (two-phase R-4 dispatch)
- `edit-protocol.md` — change detection, staleness propagation, version tagging, work scheduling

---

## Three-Layer Model (layer-model.md)

The architecture produces three distinct products from a single voxel truth:

| Product | Purpose | Derived by |
|---|---|---|
| **1. World-Space Occupancy** | Traversal, GI raymarching, shadows, picking | Chunk opaque_mask → 3D texture |
| **2. Surface Structure** | Primary raster geometry | Greedy meshing → vertex/index buffers |
| **3. Camera-Visibility Structure** | Frustum + Hi-Z occlusion culling | Occupancy summary → chunk flags + AABBs |

All three derive from GPU-resident chunk occupancy. The meshing pipeline (Product 2) and traversal pipeline (Product 1) are independent consumers of the same source data.

---

## Chunk Contract (chunk-contract.md)

The chunk is the authoritative runtime unit. Three-layer ownership:

**Authoritative (canonical source of truth):**
- `opaque_mask: [u64; CS_P²]` — bitpacked 3D occupancy
- `materials: PaletteStore` — per-voxel material IDs
- `coord: ChunkCoord`
- `data_version: u32`

**Derived (rebuilt from authoritative, never edited directly):**
- `mesh` — greedy mesh output
- `face_masks` — face visibility bitmasks
- `state` — residency/dirty state

**Planned derived:**
- `occupancy_summary` — per-8³ brick coarse occupancy
- `chunk_flags` — GPU-side visibility flags
- `aabb` — world-space bounding box

---

## ADR-0010: Radiance Cascades — Proposed

**Chosen variant:** Hybrid screenspace probes + world-space voxel raymarching (Sannikov §4.5).

- Probes on depth buffer — screen-proportional memory (~128 MB at 1080p + 4 cascades vs ~4 GB world-space)
- Rays march through `opaque_mask` 3D texture — captures off-screen light
- `MaterialDef.emissive` drives GI light sources
- Performance target: 4-8ms per frame with temporal reprojection
- Occupancy upload: 3D `r32uint` texture for cascades; storage buffer for meshing

---

## ADR-0011: Hybrid GPU-Driven Pipeline — Proposed

**Decision:** Chunk rendering → custom WebGPU pipeline. Non-chunk geometry (debug, grid, UI) → Three.js overlay.

**Convergence argument:** Radiance cascades, Hi-Z culling, and mesh culling all independently require: app-owned depth texture + compute between passes + indirect draw. One infrastructure unblocks all three.

**Status:** Proposed. Renderer worker stub (`src/renderer/renderer.worker.ts`) is the entry point. `pipeline-stages.md` is the detailed per-stage spec.

---

## Pipeline Stages (summary — see pipeline-stages.md for buffer ownership detail)

**Ingest (driven by dirty chunks):**
```
I-1  Voxelization          OBJ → GPU voxel grid
I-2  Chunk occupancy upload opaque_mask → GPU 3D texture
I-3  Summary rebuild       occupancy_summary, chunk_flags, AABBs
I-4  Greedy mesh rebuild   changed chunks → vertex/index buffers in pool
```

**Per-frame render:**
```
R-1  Depth prepass          opaque chunks → depth texture (owned GPUTexture)
R-2  Hi-Z pyramid           compute → mip chain from depth
R-3  Chunk occlusion cull   compute → indirect draw args (per-chunk + per-meshlet)
R-4  Main color pass        indirect draw chunks with material + lighting
R-5  Radiance cascade build compute (high → low cascade)
R-6  Cascade merge          compute (back-to-front)
R-7  GI application         fragment — diffuse + specular from cascade 0
R-8  Three.js overlay       debug helpers, axes, UI sprites
```

---

## Key Constraints

- WebGPU required for GI and Hi-Z. WebGL2 fallback: Three.js renders everything, no GI.
- Chunk occupancy must be GPU-resident as a 3D texture (incremental updates on dirty chunks)
- `@web` alias in `vite.config.ts` is transitional — removed when renderer worker owns the frame loop
- Depth prepass is first concrete deliverable (prerequisite for both Hi-Z and radiance cascades)

---

## Reference

- Authoritative spec: `docs/Resident Representation/` (symlinked at `memory/Resident Representation/`)
- ADR-0010: `docs/adr/0010-radiance-cascades.md`
- ADR-0011: `docs/adr/0011-hybrid-gpu-driven.md`
- WASM boundary protocol: `docs/architecture/wasm-boundary-protocol.md`
- Renderer worker entry: `apps/gestalt/src/renderer/renderer.worker.ts`
- Sannikov paper: `docs/RadianceCascades.pdf`
