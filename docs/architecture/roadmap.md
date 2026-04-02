# Gestalt Roadmap

**Type:** reference
**Status:** current
**Date:** 2026-03-22

> Phased implementation plan for the GPU-resident voxel rendering pipeline. Each phase has concrete deliverables, verification criteria, and debug visualization support.

---

## Project Identity

Gestalt is a **mesh-to-voxel viewer** and a **reusable GPU pipeline toolkit**.

A user loads a 3D mesh (OBJ), Gestalt voxelizes it, and renders the voxelized result with high-quality GPU-driven rendering including global illumination. The pipeline is designed to be reusable across projects — the rendering architecture is the product, not just the application.

---

## Phase 1 — GPU Foundation

**Goal:** Prove the Rust/WASM → WebGPU worker path works. Get pixels on screen from GPU-resident chunk data.

### Deliverables

| # | What | Crate/File | Depends on |
|---|---|---|---|
| 1.1 | Rust/WASM owns GPUDevice, presents to OffscreenCanvas | `crates/wasm_renderer/` | Worker + OffscreenCanvas transfer (done) |
| 1.2 | GPU chunk pool: allocate slots, create buffers (occupancy atlas, palette, coord, flags) | `wasm_renderer/pool.rs` | 1.1 |
| 1.3 | Procedural test scene: fill chunk occupancy from Rust (solid room + sphere + emissive clusters) | `wasm_renderer/scene.rs` | 1.2 |
| 1.4 | I-2 upload: write chunk data to GPU buffers | `wasm_renderer/passes/upload.rs` | 1.2, 1.3 |
| 1.5 | I-3 summary rebuild: compute pass derives flags, AABB, occupancy summary | `wasm_renderer/passes/summary.rs` + `summary_rebuild.wgsl` | 1.4 |
| 1.6 | R-1 greedy mesh: compute pass writes vertex/index pool | `wasm_renderer/passes/mesh_rebuild.rs` + `mesh_rebuild.wgsl` | 1.4, 1.5 |
| 1.7 | R-2 depth prepass: render all chunks depth-only | `wasm_renderer/passes/depth.rs` + `depth.wgsl` | 1.6 |
| 1.8 | R-5 color pass: render chunks with material lookup, placeholder ambient | `wasm_renderer/passes/color.rs` + `color.wgsl` | 1.7 |
| 1.9 | Camera: perspective projection + orbit via SetCamera command | `wasm_renderer/camera.rs` | 1.1 |

### Verification

| Check | Method | What you see |
|---|---|---|
| Buffers allocated correctly | GPU validation — zero WebGPU errors in console | Clean console |
| Occupancy data correct | Debug mode: render occupancy as white cubes (one per occupied voxel) | The room/sphere shape visible as a voxel cloud |
| Summary rebuild correct | Debug mode: color chunks by `is_empty` flag (green = occupied, dim = empty) | Only shell/sphere/emissive chunks colored |
| AABB correct | Debug mode: render chunk AABBs as wireframe boxes | Tight boxes around occupied regions |
| Greedy mesh correct | Color pass renders with flat shading | Smooth merged surfaces, no cracks between quads |
| Depth prepass works | Debug mode: render depth buffer as grayscale | Continuous depth gradient, no holes |
| Camera works | Orbit mouse drag + zoom | Smooth orbit around scene center |

### Done when

The procedural test scene (room + sphere + emissive voxels) renders at 60fps with correct geometry, flat-shaded materials, and all debug modes functional. Zero GPU validation errors.

**Note (2026-03-24):** Phase 1 implementation discovered an OffscreenCanvas frame tearing defect — partial frames visible during camera motion. See ADR-0014 for the investigation and resolution. The "worker thread only" constraint from ADR-0013 was revised: GPU rendering moves to the main thread, worker does CPU-only computation. Phase 1.5 implements this refactor.

---

## Phase 1.5 — Viewport Architecture Refactor

**Goal:** Move GPU rendering to the main thread to eliminate OffscreenCanvas frame tearing. Keep worker for CPU computation.

### Deliverables

| # | What | Depends on |
|---|---|---|
| 1.5.1 | Load wasm_renderer on main thread (not worker) | ADR-0014 |
| 1.5.2 | HTMLCanvasElement with main-thread WebGPU context | ADR-0014 |
| 1.5.3 | Main-thread rAF render loop (replace worker rAF) | 1.5.1, 1.5.2 |
| 1.5.4 | Direct WASM calls for camera/resize/mode (remove binary protocol for GPU ops) | 1.5.3 |
| 1.5.5 | Phi Viewport integration (canvas managed outside Svelte reactivity) | 1.5.2 |
| 1.5.6 | Worker reduced to CPU-only role (keep for Phase 2 OBJ parsing) | 1.5.4 |

### Verification

| Check | Method |
|---|---|
| No frame tearing | Orbit quickly, freeze — every frame complete |
| Same visual output | Solid, wireframe, normals, depth modes all correct |
| 74 tests still pass | `cargo test` — platform-independent code unchanged |
| No WebGPU errors | Clean console during orbit/resize |

### Done when

Same scene renders identically to Phase 1, with zero frame tearing during camera motion. All render modes functional. Worker is dormant (no GPU tasks) but available for Phase 2 CPU work.

---

## Phase 2 — Real Mesh Loading + Culling

**Goal:** Load a real OBJ file, voxelize it, and render with Hi-Z occlusion culling.

### Deliverables

| # | What | Crate/File | Depends on |
|---|---|---|---|
| 2.1 | OBJ parser in Rust/WASM (replaces TS stub) | `crates/wasm_renderer/` or `crates/wasm_obj_loader/` port | Phase 1 |
| 2.2 | I-1 voxelization: mesh → chunk occupancy (GPU compute or CPU initial impl) | `wasm_renderer/passes/voxelize.rs` | 2.1 |
| 2.3 | Material extraction: triangle materials → palette + material table | `wasm_renderer/materials.rs` | 2.1, 2.2 |
| 2.4 | R-3 Hi-Z pyramid build | `hiz_build.wgsl` | Phase 1 (R-2) |
| 2.5 | R-4 occlusion cull (chunk-level): AABB vs Hi-Z test, write indirect draw buffer | `occlusion_cull.wgsl` | 2.4 |
| 2.6 | R-5 update: indirect draw from R-4 output (replaces direct draw) | `color.wgsl` update | 2.5 |
| 2.7 | Fetch + load UI: user picks a model, worker fetches and voxelizes | Protocol command + Svelte UI | 2.1, 2.2 |

### Verification

| Check | Method | What you see |
|---|---|---|
| OBJ parsed correctly | Load cube.obj → correct 8-vertex box | A cube |
| Voxelization correct | Load bunny.obj → recognizable bunny shape in voxels | Bunny silhouette |
| Materials map correctly | Load sponza.obj → different materials on walls/floor/columns | Distinct colors per material region |
| Hi-Z pyramid correct | Debug mode: visualize Hi-Z mip levels | Coarser depth at each level, max-reduction visible |
| Occlusion cull works | Position camera behind the bunny → chunks behind it culled | Draw call counter drops when facing occluder |
| Indirect draw works | Same visual as Phase 1 direct draw, but via indirect buffer | No visual difference — performance difference |

### Done when

Load `bunny.obj` (or `teapot.obj`), voxelize it, and render the voxelized mesh with occlusion culling active. Draw call count visibly drops when the camera faces a large occluder. Hi-Z debug mode shows a correct mip pyramid. A model picker in the UI lets you switch between models.

---

## Phase 3 — Global Illumination

**Goal:** Radiance cascades producing visible GI — light bouncing off surfaces, colored shadows, emissive voxels illuminating nearby geometry.

### Deliverables

| # | What | Crate/File | Depends on |
|---|---|---|---|
| 3.1 | Traversal DDA: `traceFirstHit` and `traceSegments` in WGSL | `dda.wgsl` | Phase 2 (occupancy atlas in GPU) |
| 3.2 | Cascade 0 build: place probes on depth surface, trace rays through occupancy | `cascade_build.wgsl` | 3.1, Phase 1 (R-2 depth) |
| 3.3 | Cascade atlas: texture array, rgba16float, allocate + bind | `wasm_renderer/passes/cascade.rs` | 3.2 |
| 3.4 | R-5 update: sample cascade_atlas_0 in fragment shader, apply as GI term | `color.wgsl` update | 3.3 |
| 3.5 | Multi-cascade: build levels 1-3, merge back-to-front | `cascade_merge.wgsl` | 3.2 |
| 3.6 | Temporal reprojection: reuse previous frame's cascade data | `cascade_build.wgsl` update | 3.5 |
| 3.7 | Emissive materials: voxels with emissive > 0 emit light into cascades | Already in material_table — cascade build reads it | 3.2, Phase 2 (materials) |

### Verification

| Check | Method | What you see |
|---|---|---|
| DDA traversal correct | Debug mode: fire a ray from camera center, visualize hit point as a colored dot | Dot lands on the correct surface |
| Cascade 0 works | Place emissive voxels near a wall → wall receives colored light | Single-bounce illumination visible |
| Multi-cascade works | Move emissive source far from wall → light still reaches (via higher cascade levels) | Long-range GI, softer falloff |
| Temporal stability | Orbit camera slowly → GI doesn't flicker or shimmer | Smooth temporal blending |
| Contact shadows | Small gap between two surfaces → shadow in the gap | Ambient occlusion-like darkening |
| Colored light | Red emissive voxels → red light on nearby white surfaces | Color bleeding |

### Done when

Load a scene with emissive voxels. GI is visually correct: light bounces off surfaces, emissive materials cast colored light, contact shadows appear in crevices. Camera orbit is temporally stable (no shimmer). The GI contribution is clearly visible when toggled on/off via render mode.

---

## Phase 4 — Edit Protocol + Interactivity

**Goal:** Place and remove voxels, see the result update in real-time. The pipeline handles dirty tracking, incremental rebuilds, and budgeted work.

### Deliverables

| # | What | Crate/File | Depends on |
|---|---|---|---|
| 4.1 | Edit kernel: set/clear individual voxels in occupancy atlas | `edit.wgsl` | Phase 1 (occupancy atlas) |
| 4.2 | Dirty tracking: edit kernel sets dirty_chunks + boundary_touch_mask | `edit.wgsl` | 4.1 |
| 4.3 | Propagation pass: expand dirty to stale flags + neighbor dirty | `propagation.wgsl` | 4.2 |
| 4.4 | Compaction pass: scan stale bits → rebuild queues | `compaction.wgsl` | 4.3 |
| 4.5 | Budgeted R-1: process N chunks from mesh_rebuild_queue per frame | `mesh_rebuild.rs` update | 4.4, Phase 1 (R-1) |
| 4.6 | Budgeted I-3: process N chunks from summary_rebuild_queue per frame | `summary.rs` update | 4.4, Phase 1 (I-3) |
| 4.7 | Version stamping: mesh_version checked at swap time | `wasm_renderer/pool.rs` | 4.5 |
| 4.8 | Edit commands: new protocol opcodes for SetVoxel, ClearVoxel | `protocol.ts` + `RendererBridge.ts` | 4.1 |
| 4.9 | Edit UI: click in viewport to place/remove voxels | Svelte panel + raycasting | 4.8, Phase 3 (traceFirstHit) |

### Verification

| Check | Method | What you see |
|---|---|---|
| Single voxel edit | Click to place a voxel → appears next frame | Immediate visual feedback |
| Dirty propagation | Place voxel on chunk boundary → neighbor mesh updates too | No seam at chunk borders |
| Budget limiting | Place 1000 voxels at once → rebuilds spread over multiple frames | Smooth framerate during large edits |
| Version correctness | Edit a chunk mid-rebuild → stale mesh discarded, re-queued | No visual glitch from stale geometry |
| GI update | Place emissive voxel → cascade rebuilds → nearby surfaces light up | GI responds to edits |
| Undo | Remove placed voxel → geometry and GI revert | Clean removal, no artifacts |

### Done when

Interactive voxel editing at 60fps. Place/remove voxels with immediate visual feedback. Chunk boundary edits propagate correctly. Large edits are budget-limited without framerate drops. GI updates in response to edits.

---

## Phase 5 — Meshlets + Advanced Culling

**Goal:** Fine-grained visibility culling at sub-chunk granularity. Performance scales with visible complexity, not world size.

### Deliverables

| # | What | Depends on |
|---|---|---|
| 5.1 | Meshlet builder (Option S: 8³ subchunk grid) | Phase 4 (R-1 mesh rebuild) |
| 5.2 | Meshlet descriptor pool + range table | 5.1 |
| 5.3 | R-4 phase 2: meshlet-level Hi-Z cull | 5.2, Phase 2 (Hi-Z) |
| 5.4 | Meshlet version tracking + stale_meshlet | 5.1, Phase 4 (edit protocol) |
| 5.5 | R-9 debug modes: wireframe, normals, chunk state, occupancy heatmap | Phase 1+ |

### Verification

| Check | Method | What you see |
|---|---|---|
| Meshlet cull works | Camera facing a wall with objects behind → meshlets behind wall culled | Triangle count drops dramatically |
| Fallback works | Meshlet version mismatch → chunk-level draw used instead | No visual pop when meshlets are rebuilding |
| Debug wireframe | R-9 wireframe mode | Mesh structure visible, merged quads outlined |
| Chunk state view | R-9 chunk state mode | Chunks colored by lifecycle state |

---

## Phase 6 — Streaming + Variable Allocation

**Goal:** Worlds larger than GPU memory. Load/evict chunks on demand as camera moves.

### Deliverables

| # | What | Depends on |
|---|---|---|
| 6.1 | Variable mesh pool allocation (freelist) | Phase 5 |
| 6.2 | LRU eviction policy | 6.1 |
| 6.3 | Async chunk loading pipeline (CPU → GPU) | 6.1, 6.2 |
| 6.4 | Camera-distance prioritized loading | 6.3 |
| 6.5 | Pool compaction (defragmentation) | 6.1 |

### Verification

| Check | Method | What you see |
|---|---|---|
| Streaming works | Fly camera across a world larger than MAX_SLOTS | Chunks load ahead, evict behind |
| No pop-in | Chunks load before entering viewport | Seamless transitions |
| No stalls | Compaction doesn't cause frame drops | Consistent 60fps |
| Memory bounded | GPU memory stays within budget | No OOM crashes |

---

## Phase 7 — LOD + Material Atlas

**Goal:** Far-field rendering quality and surface detail.

### Deliverables

| # | What | Depends on |
|---|---|---|
| 7.1 | LOD Option C: point cloud for distant chunks | Phase 6 |
| 7.2 | LOD transitions: greedy mesh → point cloud at distance threshold | 7.1 |
| 7.3 | Texture atlas: 16×16 tiles, 4096 textures | Phase 2 (material system) |
| 7.4 | UV generation during greedy mesh | 7.3, Phase 1 (R-1) |

### Verification

| Check | Method | What you see |
|---|---|---|
| LOD transitions smooth | Fly toward distant chunks | Points → mesh transition without pop |
| Texture detail visible | Apply textured materials | Surface detail on voxel faces |
| Memory savings | LOD reduces mesh memory for distant chunks | More chunks visible at once |

---

## Dependency Graph

```
Phase 1 ─── GPU Foundation
  │         (procedural scene, depth, color, camera)
  │
Phase 2 ─── Real Mesh + Culling
  │         (OBJ load, voxelize, Hi-Z, indirect draw)
  │
Phase 3 ─── Global Illumination
  │         (DDA traversal, cascades, emissive lighting)
  │
Phase 4 ─── Edit Protocol
  │         (dirty tracking, incremental rebuild, interactive editing)
  │
Phase 5 ─── Meshlets + Debug Viz
  │         (sub-chunk culling, R-9 debug modes)
  │
Phase 6 ─── Streaming
  │         (LRU eviction, variable pool, async loading)
  │
Phase 7 ─── LOD + Material Atlas
            (far-field rendering, surface textures)
```

Each phase is independently valuable. Phase 1 produces a working renderer. Phase 2 makes it useful (load real meshes). Phase 3 makes it beautiful (GI). Phase 4 makes it interactive. Phases 5-7 make it scalable.

---

## Cross-Phase Verification Infrastructure

These capabilities are available at every phase and grow over time:

| Capability | Available from | How |
|---|---|---|
| GPU validation | Phase 1 | Zero WebGPU errors in console |
| Frame timing | Phase 1 | SharedArrayBuffer ring buffer → PerformancePanel |
| Depth debug | Phase 1 | Grayscale depth visualization |
| AABB wireframe | Phase 1 | Chunk bounding boxes |
| Occupancy cloud | Phase 1 | Raw voxel point rendering |
| Draw call counter | Phase 2 | DiagCounters in PerformancePanel |
| Hi-Z mip viewer | Phase 2 | Mip level selection in debug panel |
| GI on/off toggle | Phase 3 | Render mode toggle |
| DDA ray visualizer | Phase 3 | Fire test ray, show hit point |
| Edit heatmap | Phase 4 | Show recently-edited chunks |
| Meshlet wireframe | Phase 5 | Sub-chunk cluster boundaries |
| Chunk state coloring | Phase 5 | Lifecycle state visualization |
| Streaming budget monitor | Phase 6 | Load/evict counters, memory gauge |
| LOD level coloring | Phase 7 | Color by LOD level |

---

## Legacy Crate Reuse Guide

The legacy Rust crates contain valuable algorithms. The rule: **reference the algorithms, don't import the architecture.** The old WASM bindings, Three.js glue, and JS interop are discarded. The core math and data structures transfer.

### `crates/greedy_mesher/` — Binary Greedy Meshing (2,207 lines)

**Algorithms to port:**
- `cull.rs` — bitwise face culling via u64 neighbor checks. Pure bit-twiddling, no dependencies. This is the reference for `mesh_rebuild.wgsl` (R-1).
- `core.rs` + merge modules — greedy merge: horizontal runs via bitmask prefix, vertical extend. The WGSL port should produce identical output for the same `opaque_mask` input.
- `expand.rs` — quad descriptor unpacking to vertex positions/normals/indices. 64-bit packed format (6-bit coords + 6-bit dimensions + 16-bit material).

**Keep as test oracle:**
- Run the Rust mesher and the WGSL mesher on the same `BinaryChunk` input. Compare vertex/index output. Any divergence is a bug in the WGSL port.
- The existing test suite (single voxels, cubes, merge efficiency, boundary conditions) defines the correctness contract.

**Constants that transfer directly:**
- `CS_P = 64`, `CS = 62`, `CS_P2 = 4096`, `CS_P3 = 262144`
- `MATERIAL_EMPTY = 0`, `MATERIAL_DEFAULT = 1`
- The 1-voxel padding convention (64³ storage, 62³ usable) is fundamental to the architecture, not legacy.

**Phase mapping:** Phase 1, deliverable 1.6 (R-1 mesh rebuild).

---

### `crates/voxelizer/` — GPU Mesh-to-Voxel Rasterization (713 lines)

**Algorithms to port:**
- `csr.rs` — Compressed Sparse Row spatial indexing. Builds tile/brick triangle lists for dispatch. Pure CPU, no GPU deps. Transfers directly to the new I-1 voxelization pass.
- `reference_cpu.rs` — SAT-based triangle-box overlap test. Keep as validation oracle — run on the same mesh, compare occupancy output against the GPU voxelizer.
- `core.rs` — `CompactVoxel` struct `[vx: i32, vy: i32, vz: i32, material: u32]` (16 bytes, `#[repr(C)]`). This is the courier format between voxelizer output and chunk pool upload (I-1 → I-2).

**Architecture to discard:**
- The `wgpu` pipeline setup (`gpu/mod.rs`, `gpu/pipelines.rs`) — rewrite for WebGPU in the worker. The *algorithm* (conservative rasterization, epsilon expansion) transfers; the *API calls* don't.
- Dense-to-sparse conversion was a workaround for the old pipeline's memory constraints. The new pipeline uses the chunk pool directly.

**Phase mapping:** Phase 2, deliverable 2.2 (I-1 voxelization).

---

### `crates/wasm_obj_loader/` — OBJ Parser (190 lines)

**Absorb into `wasm_renderer`:**
- The `parse_obj` function (line parser for `v`, `f`, `usemtl`) is correct and handles fan triangulation + material groups. Absorb directly — don't rewrite from scratch, but don't keep it as a separate crate.
- The `transform_matrix` and `multiply_mat4` functions — replace with `glam` crate equivalents. The hand-rolled matrix math is correct but unnecessary when `glam` is available.

**Phase mapping:** Phase 2, deliverable 2.1 (OBJ parser in Rust/WASM).

---

### Crates to discard (reference only)

| Crate | Why discard | What to reference |
|---|---|---|
| `wasm_voxelizer` | Thin JS interop wrapper over `voxelizer`. The new pipeline calls Rust directly from the worker, not via JS. | Fallback heuristic (when to use CPU vs GPU path) |
| `wasm_greedy_mesher` | Thin JS interop wrapper over `greedy_mesher`. Same reason. | The `bytemuck` usage pattern for GPU buffer uploads |
| `wasm_webgpu_demo` | Tutorial-level compute shader example. | WGSL workgroup size patterns, binding layout conventions |

---

### Validation Strategy: Legacy as Sanity Check, Spec as Ground Truth

The legacy crates produced good visual results but were never formally verified. They may contain subtle bugs (off-by-one in padding, missed thin triangles, edge cases in merge). **The spec documents in `data/` and `stages/` are the source of truth — not the legacy code.**

Validation has three tiers:

**Tier 1 — Spec invariants (authoritative):**
Test the new implementation directly against the formal invariants defined in the spec docs (OCC-1, FLG-1, PAL-1, etc.). These are the correctness criteria. If the spec says "every occupied voxel is inside the AABB" (AABB-2), test that. The legacy code is irrelevant here.

**Tier 2 — Legacy comparison (sanity check):**
Run both the legacy Rust implementation and the new WGSL/Rust implementation on the same input. If they agree, good — confidence is higher. If they disagree, **investigate which one is correct against the spec.** The legacy output is not assumed correct. Common disagreement sources:
- Padding/boundary handling (the spec is now explicit; the legacy code may not match)
- Material palette ordering (the spec defines dominance ordering; the legacy may not)
- Degenerate inputs (empty chunks, single voxels, 3D checkerboard — the legacy may not handle all)

**Tier 3 — Visual inspection (human judgment):**
Render the output and look at it. Does the bunny look like a bunny? Are there cracks between chunks? Is the GI plausible? This catches classes of bugs that neither spec invariants nor legacy comparison can detect (e.g., correct but ugly merge patterns, correct but noisy GI).

All three tiers are needed. Tier 1 alone misses visual quality issues. Tier 2 alone inherits legacy bugs. Tier 3 alone is subjective and non-reproducible.

The legacy crates are **never called from the production pipeline.** They exist in the repo as one input to validation — alongside the spec and human judgment.

---

## See Also

- [demo-renderer](../Resident Representation/demo-renderer.md) — Phase 1 procedural scene spec
- [ADR-0013](../adr/0013-full-webgpu-worker-pipeline.md) — Architecture decision for the worker pipeline
- [pipeline-stages](../Resident Representation/pipeline-stages.md) — Full stage specifications
- [data/INDEX](../Resident Representation/data/INDEX.md) — All data structure specs
- [stages/INDEX](../Resident Representation/stages/INDEX.md) — All stage specs
- [tests/INDEX](../Resident Representation/tests/INDEX.md) — All consistency tests
- [underspecified-decisions-report](../../reference/underspecified-decisions-report.md) — Resolved design decisions
