# Implementation Status — Minimum Viable Systems
## Dependency Graph

```
Goal A (buffer reuse) ◀──────── standalone, original three.js is enough
    │
    │ if GPU-driven path chosen, merges into:
    ▼
Goal B (custom pipeline) ◀──── prerequisite for the stuff below
    │
    ├──────────────────────┐
    ▼                      ▼
Goal D (GI)          Goal E (culling)
    │                      │
    └──────────┬───────────┘
               ▼
         Full pipeline

Goal C (OBJ→chunks) ◀──────── independent from everything else
```

---

## What's Closest to Done

**Goal C is the cheapest to complete.** The voxelizer and chunk manager both
work independently — the missing piece is the ~200 lines of glue code to
compact voxelizer output and feed it into ChunkManager. Plus Goal A for
proper buffer reuse on the render side.

**Goal A is the most impactful for the existing codebase.** It closes 4 of
the 5 unmet requirements without requiring any GPU pipeline work. It can be
done with Three.js mesh pooling alone.

**Goal B is the gate to the future.** Nothing in Goals D or E can start until
there's a custom render pass writing to an app-owned depth texture. This is
the minimum infrastructure that enables the GPU-driven architecture.

---

### Voxel Data (REQ-IN-*)

| Req | Description | Met? | By what |
|-----|-------------|------|---------|
| REQ-IN-001 | Boolean occupancy per voxel | **Yes** | `BinaryChunk.opaque_mask` — u64 column bitmasks |
| REQ-IN-002 | Material ID per voxel | **Yes** | `PaletteMaterials` — bitpacked palette per chunk |
| REQ-IN-003 | Optional color per voxel | **Partial** | Voxelizer stores `color_rgba` per brick; no palette color path yet |
| REQ-IN-004 | Grid transform (origin + voxel_size) | **Yes** | `VoxelGridSpec` in voxelizer, `ChunkCoord` math in mesher |
| REQ-IN-005 | Deterministic output | **Yes** | Same input → byte-identical mesh (tested) |

### Surface Extraction (REQ-SURF-*)

| Req | Description | Met? | By what |
|-----|-------------|------|---------|
| REQ-SURF-001 | Face culling at solid/empty boundaries | **Yes** | `cull.rs` — bitwise face culling, 8 tests |
| REQ-SURF-002 | No interior geometry | **Yes** | Implicit from bitmask cull algorithm |
| REQ-SURF-003 | Greedy meshing (merge coplanar) | **Yes** | `merge/` — all 3 axes, 15+ tests |
| REQ-SURF-004 | Correct winding (CCW from outside) | **Yes** | `expand.rs` — per-face-direction winding, tested |

### Geometry Output (REQ-GEO-*)

| Req | Description | Met? | By what |
|-----|-------------|------|---------|
| REQ-GEO-001 | Valid BufferGeometry with positions | **Yes** | `MeshOutput.positions` → Three.js BufferGeometry |
| REQ-GEO-002 | Indexed geometry | **Yes** | `MeshOutput.indices` |
| REQ-GEO-003 | Per-face normals | **Yes** | `MeshOutput.normals` |
| REQ-GEO-004 | Optional vertex colors | **No** | Not implemented — colors exist in voxelizer output but aren't mapped to vertex colors |
| REQ-GEO-005 | Optional material groups | **Partial** | `MeshOutput.material_ids` exists per-quad; no Three.js material group splitting yet |

### Chunk Management (REQ-CHUNK-*)

| Req | Description | Met? | By what |
|-----|-------------|------|---------|
| REQ-CHUNK-001 | 64³ chunks (62³ usable) | **Yes** | `CS_P=64, CS=62` in `core.rs` |
| REQ-CHUNK-002 | Dirty marking with boundary propagation | **Yes** | `DirtyTracker` + `mark_dirty_with_neighbors()` |
| REQ-CHUNK-003 | Deduped rebuild queue | **Yes** | `RebuildQueue` with `in_queue: HashSet` |
| REQ-CHUNK-004 | Budgeted rebuilds | **Yes** | `ChunkManager.update()` with frame budget config |
| REQ-CHUNK-005 | Camera-distance prioritization | **Partial** | `RebuildQueue` uses priority, but camera distance isn't wired as the priority source yet |
| REQ-CHUNK-006 | Snapshot/versioning | **Yes** | `data_version` on Chunk, version-checked mesh swap |

### Rendering (REQ-RENDER-*)

| Req | Description | Met? | By what |
|-----|-------------|------|---------|
| REQ-RENDER-001 | Stable Mesh objects (reuse) | **No** | Current path creates new Three.js meshes per output; no mesh pool/reuse |
| REQ-RENDER-002 | Preallocated buffers with drawRange | **No** | Each chunk gets its own `BufferGeometry` allocated on rebuild |
| REQ-RENDER-003 | Double-buffered geometry swaps | **No** | Mesh data is copied directly, no double-buffering |
| REQ-RENDER-004 | Clipping planes | **No** | Not implemented |
| REQ-RENDER-005 | WebGL + WebGPU compatible | **Yes** | Three.js WebGPURenderer with WebGL2 fallback |

### Performance (REQ-PERF-*)

| Req | Description | Met? | By what |
|-----|-------------|------|---------|
| REQ-PERF-001 | No per-frame rebuilds for static | **Yes** | Only dirty chunks are rebuilt |
| REQ-PERF-002 | Incremental updates | **Yes** | Per-chunk dirty + rebuild |
| REQ-PERF-003 | Main thread responsive | **Partial** | Worker exists for chunk manager, but meshing still blocks worker thread (no work splitting within a chunk) |
| REQ-PERF-004 | Memory budget awareness | **Yes** | `MemoryBudget` + LRU eviction in ChunkManager |

### Debugging (REQ-DEBUG-*)

| Req | Description | Met? | By what |
|-----|-------------|------|---------|
| REQ-DEBUG-001 | Face count by axis | **Yes** | `mesh_chunk_with_stats()` returns per-axis counts |
| REQ-DEBUG-002 | Triangle/vertex count per chunk | **Yes** | Stats in `MeshOutput`, displayed in viewer |
| REQ-DEBUG-003 | Bounding box visualization | **Yes** | Viewer bounds helpers |
| REQ-DEBUG-004 | Wireframe mode | **Yes** | Toggle in viewer |
| REQ-DEBUG-005 | Normals debug viz | **Partial** | Debug mesh output exists; no dedicated normals display mode in viewer |
| REQ-DEBUG-006 | Chunk state inspection | **Yes** | `WasmChunkDebugInfo` with full state + memory reporting |

---

## Summary Scorecard

| Category | Total | Met | Partial | Not Met |
|----------|-------|-----|---------|---------|
| Voxel Data (IN) | 5 | 4 | 1 | 0 |
| Surface Extraction (SURF) | 4 | 4 | 0 | 0 |
| Geometry Output (GEO) | 5 | 3 | 1 | 1 |
| Chunk Management (CHUNK) | 6 | 5 | 1 | 0 |
| Rendering (RENDER) | 5 | 1 | 0 | 4 |
| Performance (PERF) | 4 | 3 | 1 | 0 |
| Debugging (DEBUG) | 6 | 5 | 1 | 0 |
| **Total** | **35** | **25** | **5** | **5** |

**71% fully met, 14% partially met, 14% not met.**

The core algorithms work. The gaps are concentrated in the **rendering layer**
(4 of 5 unmet) and **geometry output** (vertex colors, material groups).

---

## Gap Analysis — What Must Be Built

### Gap 1: Render Object Reuse (REQ-RENDER-001, -002, -003)

**Problem:** Each chunk mesh creates a fresh Three.js `BufferGeometry` and
`Mesh`. No pooling, no preallocation, no double-buffering. This means scene
graph churn and GPU memory fragmentation as chunks rebuild.

**Minimum viable fix:** A mesh buffer pool that:
- Maintains a fixed set of `THREE.Mesh` objects (or GPU buffer slots)
- Assigns chunks to slots on load, reuses on eviction
- Swaps geometry data into existing slots (not new allocations)
- Uses `drawRange` to render partial buffers

**This is needed regardless of whether we go GPU-driven or stay Three.js.**
The requirement is about buffer lifecycle, not about which renderer.

**Two paths to satisfy this:**
1. **Three.js path:** `ChunkMeshPool` from `threejs-buffer-management.md` —
   preallocated `BufferGeometry` pool, double-buffered swap
2. **GPU-driven path:** `chunkBufferPool.ts` — single GPU buffer with
   per-chunk offsets, which also enables indirect draw later

Path 2 is strictly more capable (enables everything Path 1 does plus indirect
draw), so if GPU-driven rendering is the direction, build Path 2 directly.

---

### Gap 2: Custom Pipeline Foundation (REQ-RENDER-002 via GPU path)

**Problem:** To use a global GPU buffer pool, we need the ability to issue
our own draw calls against our own buffers. Three.js doesn't expose this.

**Minimum viable fix:**
1. **Shared GPUDevice** — extract from `renderer.backend.device`
2. **Depth prepass** — custom render pass writing to app-owned depth texture
3. **Color pass** — custom render pass reading depth (EQUAL test) + writing color

The depth prepass is the single piece that unlocks the most downstream work
(Hi-Z culling, radiance cascades both need it). But a color pass is needed
before the depth prepass is useful — otherwise you have depth data with nothing
consuming it for on-screen output.

**Minimum for visible output:**
```
Shared device → Buffer pool → Depth prepass → Color pass → Three.js overlay
```

---

### Gap 3: Vertex Colors (REQ-GEO-004)

**Problem:** Voxelizer produces per-voxel `color_rgba`. This data exists but
isn't mapped into `MeshOutput` or vertex attributes.

**Minimum viable fix:** Add optional `colors: Vec<f32>` to `MeshOutput`.
During quad expansion, look up the source voxel's color and emit it as a
vertex attribute. Fragment shader reads vertex color when no material atlas
is bound.

**Scope:** Rust side (`expand.rs`, `core.rs`), WASM bindings
(`wasm_greedy_mesher`), vertex format change.

---

### Gap 4: Material Groups (REQ-GEO-005)

**Problem:** `MeshOutput.material_ids` exists per-quad, but nothing splits
the index buffer into material groups for Three.js multi-material rendering
(or for atlas-based lookup in a custom shader).

**Minimum viable fix depends on rendering path:**
- **Three.js path:** Sort indices by material_id, emit `groups[]` array for
  `BufferGeometry.addGroup()`. Each group gets its own material.
- **GPU-driven path:** Pass `material_id` per-vertex to fragment shader.
  Shader indexes into a material data texture. No sorting needed.

GPU-driven path is simpler here — no index buffer reordering required.

---

### Gap 5: Clipping Planes (REQ-RENDER-004)

**Problem:** No cross-section/slicing support.

**Minimum viable fix:** Pass a clip plane uniform to the fragment shader,
`discard` fragments on the wrong side. One uniform vec4 (plane equation).

**Scope:** Trivial once a custom fragment shader exists (Gap 2). Nearly
impossible to do correctly through Three.js's clipping plane system if
chunks are in a global buffer pool.

---

### Gap 6: Camera-Distance Priority (REQ-CHUNK-005)

**Problem:** `RebuildQueue` accepts priority values, but nothing computes
priority from camera distance.

**Minimum viable fix:** When marking a chunk dirty, compute
`distance(camera_pos, chunk_center)` and pass as priority to
`RebuildQueue.enqueue()`. Closer chunks rebuild first.

**Scope:** TypeScript only — the priority plumbing exists in Rust, just
needs the camera-distance input from the JS side.

---

### Gap 7: Voxelizer → ChunkManager Pipeline (not a formal REQ, but needed for OBJ→render)

**Problem:** The voxelizer produces sparse voxel data. The chunk manager
consumes `set_voxel()` calls. There's no bridge between them — currently the
testbed shows voxelizer output as raw cubes/points, not as greedy-meshed chunks.

**Minimum viable fix (ADR-0009):**
1. Compact voxelizer output to `(vx, vy, vz, material_id)` tuples
2. Group by `div_euclid(vx, CS)` → per-chunk batches
3. Bulk `set_voxel_raw()` into ChunkManager
4. Let normal dirty→rebuild→render flow take over

---

## Minimum Viable Systems — By Goal

### Goal A: "Greedy-meshed chunks render correctly with buffer reuse"

Satisfies: REQ-RENDER-001 through -003, closes the biggest gap cluster.

**Systems needed:**
1. Global mesh buffer pool (or Three.js mesh pool)
2. Slot-based allocation with drawRange
3. Double-buffered swap on mesh rebuild

**Already have:** ChunkManager, MeshOutput, meshing pipeline, Three.js renderer

---

### Goal B: "Custom rendering pipeline with depth"

Satisfies: Foundation for REQ-RENDER-002 (preallocated GPU buffers),
REQ-RENDER-004 (clipping via shader), and enables Hi-Z + cascades.

**Systems needed:**
1. Shared GPUDevice extraction
2. Camera UBO (VP matrix, position)
3. Global mesh buffer pool (GPU-side)
4. Depth prepass render pass + WGSL shader
5. Custom color pass + WGSL shader
6. Three.js overlay compositing

**Already have:** WebGPU device access pattern (`webgpuDiagnostics.ts`),
camera data (Three.js `PerspectiveCamera`), mesh data (MeshOutput)

---

### Goal C: "OBJ loads render as greedy-meshed chunks"

Satisfies: Complete data pipeline from input to screen.

**Systems needed:**
1. Compact voxelizer output → `(vx,vy,vz,material)` tuples
2. Group-by-chunk + bulk ChunkManager insertion
3. Everything from Goal A (buffer reuse for rendering)

**Already have:** OBJ parser, GPU voxelizer, ChunkManager, greedy mesher

---

### Goal D: "Global illumination via radiance cascades"

Satisfies: The visual feature that motivated the GPU-driven architecture.

**Systems needed:**
1. Everything from Goal B (custom pipeline with depth)
2. Occupancy 3D texture upload (opaque_mask → r32uint 3D tex)
3. Cascade raymarch compute shader
4. Cascade merge compute shader
5. Temporal reprojection
6. Fragment shader GI application

**Already have:** BinaryChunk.opaque_mask (acceleration structure), depth
prepass output (from Goal B), custom fragment shader (from Goal B)

---

### Goal E: "GPU occlusion culling"

Satisfies: Performance at scale.

**Systems needed:**
1. Everything from Goal B (depth prepass)
2. Chunk AABB buffer (center + extents per chunk)
3. Hi-Z pyramid build compute shader
4. Cull compute shader (AABB vs pyramid)
5. Indirect draw args buffer
6. `drawIndexedIndirect` in depth + color passes

**Already have:** Depth texture (from Goal B), ChunkCoord (for AABB derivation)

---

## See Also

- [`architecture-map.md`](architecture-map.md) — all data structures and relationships
- [`gpu-driven-rendering/INDEX.md`](gpu-driven-rendering/INDEX.md) — pipeline documentation hub
- [`gpu-driven-rendering/design/hybrid-transition.md`](gpu-driven-rendering/design/hybrid-transition.md) — phase details
- [`gpu-driven-rendering/spec/frame-graph.md`](gpu-driven-rendering/spec/frame-graph.md) — pass definitions
- [`greedy-meshing-docs/voxel-mesh-architecture.md`](greedy-meshing-docs/voxel-mesh-architecture.md) — requirements source
