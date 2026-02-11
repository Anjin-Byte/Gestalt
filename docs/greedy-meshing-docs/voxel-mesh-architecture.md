# Voxel Mesh Architecture Overview

> **[Documentation Index](INDEX.md)** - Quick navigation for all docs, reading order, and quick reference.

This document provides a high-level overview of the voxel-to-mesh rendering pipeline. Detailed specifications for each component are in separate documents.

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           VOXEL DATA LAYER                                  │
│  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐         │
│  │  Chunk Storage  │    │  Dirty Tracker  │    │  Edit History   │         │
│  │  (Authoritative)│───▶│  (Boundary-Aware)│───▶│  (Optional)     │         │
│  └─────────────────┘    └─────────────────┘    └─────────────────┘         │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           MESH GENERATION LAYER                             │
│  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐         │
│  │  Rebuild Queue  │───▶│  Greedy Mesher  │───▶│  Mesh Cache     │         │
│  │  (Deduped)      │    │  (Per-Chunk)    │    │  (Versioned)    │         │
│  └─────────────────┘    └─────────────────┘    └─────────────────┘         │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           RENDER LAYER (Three.js)                           │
│  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐         │
│  │  Buffer Manager │───▶│  Mesh Pool      │───▶│  Scene Graph    │         │
│  │  (Preallocated) │    │  (Double-Buffer)│    │  (Clipping)     │         │
│  └─────────────────┘    └─────────────────┘    └─────────────────┘         │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Core Principles

### 1. Voxel Data is Truth
- Chunk voxel data is the authoritative source
- Mesh geometry is a derived cache, regenerated when voxels change
- Never modify mesh directly; always edit voxels, then rebuild mesh

### 2. Lazy Mesh Generation
- Meshes are generated on-demand, not eagerly
- Dirty chunks are queued for rebuild
- Rebuilds are budgeted across frames to prevent jank

### 3. Boundary Awareness
- Voxel edits at chunk boundaries affect neighboring chunks
- Dirty marking must propagate to adjacent chunks when needed
- Face visibility depends on neighbors in adjacent chunks

### 4. Stable Render Objects
- Three.js Mesh objects are reused, not recreated
- Geometry buffers are swapped, not the Mesh itself
- This avoids scene graph churn and memory fragmentation

## Component Documents

| Document | Description |
|----------|-------------|
| [Development Guidelines](development-guidelines.md) | Coding standards, file organization, function design |
| [Implementation Plan](implementation-plan.md) | Phased milestones, debug tooling, timeline |
| [Architecture Addendum](architecture-addendum.md) | Boundary meshing, material strategy, coordinate systems |
| [Greedy Mesh Implementation](greedy-mesh-implementation-plan.md) | Surface extraction and greedy meshing algorithm |
| [Binary Greedy Meshing Analysis](binary-greedy-meshing-analysis.md) | Bitwise optimization techniques (10-50x speedup) |
| [Chunk Management System](chunk-management-system.md) | Dirty tracking, rebuild queue, state machine |
| [Three.js Buffer Management](threejs-buffer-management.md) | Buffer allocation, double-buffering, GPU uploads |

## Requirements Summary

### Inputs (Voxel Field)

| Requirement | Description |
|-------------|-------------|
| **REQ-IN-001** | Voxel occupancy: boolean `solid` flag per voxel |
| **REQ-IN-002** | Optional material ID (u8) per voxel |
| **REQ-IN-003** | Optional color (RGB) per voxel |
| **REQ-IN-004** | Grid transform: origin + voxel_size |
| **REQ-IN-005** | Deterministic output: same input → byte-identical mesh |

### Surface Extraction

| Requirement | Description |
|-------------|-------------|
| **REQ-SURF-001** | Face culling: emit faces only at solid/empty boundaries |
| **REQ-SURF-002** | No interior geometry: hidden faces never emitted |
| **REQ-SURF-003** | Greedy meshing: merge coplanar faces into larger quads |
| **REQ-SURF-004** | Correct winding order: CCW when viewed from outside |

### Geometry Output

| Requirement | Description |
|-------------|-------------|
| **REQ-GEO-001** | Valid BufferGeometry with position attribute |
| **REQ-GEO-002** | Indexed geometry (shared vertices) |
| **REQ-GEO-003** | Per-face normals for flat shading |
| **REQ-GEO-004** | Optional vertex colors |
| **REQ-GEO-005** | Optional material groups |

### Chunk Management

| Requirement | Description |
|-------------|-------------|
| **REQ-CHUNK-001** | Fixed chunk size of 64³ (62³ usable with 1-voxel padding) for binary meshing |
| **REQ-CHUNK-002** | Dirty marking with boundary neighbor propagation |
| **REQ-CHUNK-003** | Deduped rebuild queue |
| **REQ-CHUNK-004** | Budgeted rebuilds (N chunks per frame) |
| **REQ-CHUNK-005** | Camera-distance prioritization |
| **REQ-CHUNK-006** | Snapshot/versioning for rebuild consistency |

### Rendering

| Requirement | Description |
|-------------|-------------|
| **REQ-RENDER-001** | Stable Mesh objects (reuse, don't recreate) |
| **REQ-RENDER-002** | Preallocated buffers with drawRange |
| **REQ-RENDER-003** | Double-buffered geometry swaps |
| **REQ-RENDER-004** | Clipping planes for slicing (not geometry rebuild) |
| **REQ-RENDER-005** | Compatible with WebGL and WebGPU renderers |

### Performance

| Requirement | Description |
|-------------|-------------|
| **REQ-PERF-001** | No per-frame geometry rebuilds for static scenes |
| **REQ-PERF-002** | Incremental updates (only affected chunks) |
| **REQ-PERF-003** | Main thread remains responsive during rebuilds |
| **REQ-PERF-004** | Memory budget awareness (dispose unused) |

### Debugging

| Requirement | Description |
|-------------|-------------|
| **REQ-DEBUG-001** | Face count visualization by axis |
| **REQ-DEBUG-002** | Triangle/vertex count per chunk |
| **REQ-DEBUG-003** | Bounding box visualization |
| **REQ-DEBUG-004** | Wireframe mode |
| **REQ-DEBUG-005** | Normals debug visualization |
| **REQ-DEBUG-006** | Chunk state inspection |

## Data Flow

### Static Scene Load

```
1. Load voxel data (from file/network)
2. Partition into chunks
3. Queue all chunks for initial mesh build
4. Process rebuild queue (budgeted)
5. Upload geometries to GPU
6. Render
```

### Dynamic Edit

```
1. User edits voxel at position P
2. Determine containing chunk C
3. Mark C as dirty
4. If P is on chunk boundary, mark neighbor chunks dirty
5. Add dirty chunks to rebuild queue (dedupe)
6. Process queue (budgeted, prioritized)
7. Swap new geometry into existing Mesh
8. Render
```

### Slicing (Cross-Section View)

```
1. User adjusts slice plane
2. Update material clippingPlanes
3. Render (no geometry rebuild)
```

## Chunk Coordinate System

```
World Position → Chunk Coordinate:
  chunk_x = floor(world_x / (chunk_size * voxel_size))
  chunk_y = floor(world_y / (chunk_size * voxel_size))
  chunk_z = floor(world_z / (chunk_size * voxel_size))

Chunk Coordinate → Chunk Origin (World):
  origin_x = chunk_x * chunk_size * voxel_size
  origin_y = chunk_y * chunk_size * voxel_size
  origin_z = chunk_z * chunk_size * voxel_size
```

## Memory Layout

### Per-Chunk Data (64³ with Binary Meshing)

| Data | Size | Notes |
|------|------|-------|
| Opaque mask | 32 KB | 64×64 columns × 8 bytes (bitmask) |
| Material IDs | 256 KB | 1 byte per voxel (64³) |
| Face masks | 192 KB | 6 directions × 32 KB |
| Packed quads | Variable | 8 bytes per quad |
| Expanded mesh | Variable | ~4 verts per quad |

### Worst-Case Mesh Size

For a fully fragmented chunk (checkerboard pattern, 62³ usable):
- Max visible faces: `62³ * 6 / 2` = 714,216 faces
- Packed quads: 714,216 × 8 bytes = 5.7 MB
- Expanded vertices: 714,216 × 4 = 2,856,864 vertices
- Position data: 2,856,864 × 3 × 4 bytes = 34.3 MB
- Index data: 714,216 × 6 × 4 bytes = 17.1 MB

Typical case with greedy meshing: 10-100x smaller.

### Binary Meshing Performance

| Chunk Content | Time | Memory |
|---------------|------|--------|
| Empty | ~5 µs | ~0 |
| Solid cube | ~30 µs | ~48 bytes |
| Terrain surface | ~74 µs | ~10-50 KB |
| Complex caves | ~150 µs | ~50-200 KB |

## Next Steps

See [Implementation Plan](implementation-plan.md) for detailed phased milestones:

1. **Phase 1:** Binary greedy meshing algorithm (Rust) - 10-50x faster via bitwise operations
2. **Phase 2:** Chunk system with dirty tracking (Rust)
3. **Phase 3:** Three.js render integration (TypeScript)
4. **Phase 4:** Debug tooling and visualization
5. **Phase 5:** Optimization (workers, LOD, memory)
6. **Phase 6:** Polish and documentation
