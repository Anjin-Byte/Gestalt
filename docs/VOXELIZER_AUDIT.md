# Voxelizer Audit (Methodical Pass)

## Scope
- Focus: Rust voxelizer core + GPU pipeline + WASM bindings + JS module glue + Three.js output builder.
- Goal: document what each block is intended to do, then compare to the spec and highlight likely mismatch points.

## System Map
- Rust core: `crates/voxelizer/src/core.rs`, `crates/voxelizer/src/csr.rs`
- Rust GPU: `crates/voxelizer/src/gpu.rs` (WGSL + wgpu)
- Rust CPU reference: `crates/voxelizer/src/reference_cpu.rs`
- WASM bindings: `crates/wasm_voxelizer/src/lib.rs`
- JS module glue: `apps/web/src/modules/wasmVoxelizer.ts`
- Viewer output builders: `apps/web/src/viewer/outputs.ts`, `apps/web/src/viewer/Viewer.ts`

## File-by-File Documentation

### `crates/voxelizer/src/core.rs`
- Purpose: core data types and validation (grid/tiles/mesh/options/output).
- Key types:
  - `VoxelGridSpec`: origin + voxel_size + dims + optional world_to_grid; validation ensures positive voxel_size, dims>0, finite transforms.
  - `TileSpec`: tile/brick dimensions and derived tile counts; validates `tile_dims` product <= `max_invocations`.
  - `MeshInput`: triangle list + optional material IDs; validates finite vertices and matching material ID length.
  - `VoxelizeOpts`: epsilon + owner/color toggles (defaults to enabled).
  - `VoxelizationOutput` (dense) + `SparseVoxelizationOutput` (brick list + occupancy).
- Intended contract: grid dims in voxels, world_to_grid uses origin/voxel_size unless provided.

### `crates/voxelizer/src/csr.rs`
- Purpose: bin triangles into tile/brick CSR lists.
- Functions:
  - `build_tile_csr`: compute conservative triangle bounds in grid coords using epsilon, map to tile range, fill `tile_offsets`/`tri_indices`.
  - `build_brick_csr`: compute triangle AABB → brick grid AABB; map to brick origins, build CSR per brick.
- Invariants:
  - `tile_offsets[0] == 0`, monotonically increasing, final offset == `tri_indices.len()`.
  - Brick origins sorted by (z,y,x) for stable ordering.
- Notes: triangle degeneracy not explicitly filtered (relies on later SAT test).

### `crates/voxelizer/src/gpu.rs`
- Purpose: GPU voxelization and GPU compaction into positions.
- Key structures:
  - `GpuVoxelizer`: owns wgpu instance/device/queue + compute pipelines.
  - `Params`: uniform for voxelization (grid dims, tile dims, counts, flags).
  - `CompactParams`: uniform for compaction (brick dim/count, max_positions, origin/voxel_size).
- Passes:
  - `voxelize_surface`: dense tile-based voxelization using tile CSR.
  - `voxelize_surface_sparse`: brick-based voxelization using brick CSR.
  - `voxelize_surface_sparse_chunked`: splits brick CSR into chunks; each chunk dispatches a compute pass.
  - `compact_sparse_positions(_buffer)`: compacts occupancy → positions (vec4<f32> per voxel).
- WGSL:
  - `triangle_box_overlap` implements SAT: 9 cross-product axes + 3 box axes + triangle normal plane test.
  - Occupancy stored as `array<atomic<u32>>` (bitset), owner/color stored per voxel.
  - Brick mode uses `brick_origins` when `num_tiles_xyz` is zero.
- Intended constraints:
  - Tile/brick voxels per workgroup <= `max_compute_invocations_per_workgroup`.
  - Brick count per dispatch <= `max_compute_workgroups_per_dimension` (enforced in compaction).
  - Max storage buffer size used to bound chunk size in sparse voxelization.

### `crates/voxelizer/src/reference_cpu.rs`
- Purpose: CPU oracle with SAT triangle–AABB for validation or fallback.
- Mirrors WGSL overlap logic and uses epsilon-expanded AABB.
- Owner/color rules: min triangle index for ownership; color hashed from owner ID.

### `crates/wasm_voxelizer/src/lib.rs`
- Purpose: WASM bridge for JS:
  - `voxelize_triangles` → sparse occupancy + brick list + debug stats.
  - `voxelize_triangles_positions` → GPU compaction → positions (CPU readback).
  - `voxelize_triangles_positions_chunked` → chunked sparse → per-chunk GPU compaction.
  - `voxelize_triangles_chunked` → chunked sparse occupancy for JS to expand.
- Fallback path: if GPU result empty and mesh non-empty, CPU dense -> sparse (guarded by `MAX_FALLBACK_BYTES`).
- Logging: `set_log_enabled` toggles wasm-side logging.
- Notes:
  - `dense_to_sparse` builds bricks by scanning all dense voxels (O(Nvoxels)).
  - Chunked compaction divides `max_positions` across chunks.

### `apps/web/src/modules/wasmVoxelizer.ts`
- Purpose: JS module adapter for voxelizer + UI parameters + rendering outputs.
- Core flow:
  1. Parse OBJ or use default cube.
  2. Compute grid + voxel size (optional fit to bounds).
  3. Run voxelization based on progressive/compact/gpuCompact toggles.
  4. Build positions (either GPU compaction or JS sparse expansion).
  5. Emit `ModuleOutput` of kind `voxels`.
- Controls:
  - Grid dim, voxel size, epsilon, chunk size, render chunk, render mode (points/cubes).
  - Progressive chunking and compact toggles.
  - GPU compact for positions (readback).
- Key detail:
  - `render-chunk` is a *rendering* chunk size (voxels per InstancedMesh), not voxelization chunk size.

### `apps/web/src/viewer/outputs.ts`
- Purpose: Convert `ModuleOutput` into Three.js objects.
- Voxel render:
  - Points: chunked Points objects.
  - Cubes: instanced BoxGeometry built asynchronously (time-sliced).
- Important behaviors:
  - Cube build uses a clamped chunk size (5k..20k) regardless of input.
  - `DynamicDrawUsage` + `addUpdateRange` for instance matrices (WebGPU stability).
  - Build can be canceled via `buildToken` (prevents stale renders).

### `apps/web/src/viewer/Viewer.ts`
- Purpose: Scene integration + toggles (wireframe/unlit/grid/axes/bounds), overlays.
- `setOutputs` rebuilds output group; uses `computeStats`.
- `disposeOutputGroup` cancels async cube build and disposes materials/geometry.

## Spec Alignment (Intent vs. Code)

### Conservative triangle–AABB overlap (SAT)
- Spec: Akenine-Möller SAT (triangle normal + box axes + 9 cross-axes).
- Code: WGSL + CPU path implement the full SAT tests.
- Status: aligned.

### Tile/binning architecture
- Spec: CSR tile → triangle list; each tile processed in one workgroup.
- Code: `build_tile_csr` + `voxelize_surface` match this.
- Sparse variant: brick CSR and `voxelize_surface_sparse(_chunked)` use brick list; each brick maps to one workgroup.
- Status: aligned (brick-based sparse variant extends the spec).

### Workgroup isolation / no cross-workgroup sync
- Spec: no global barriers.
- Code: each dispatch is independent; chunking splits into multiple dispatches.
- Status: aligned.

### Atomic usage and storage layout
- Spec: atomic<u32> for bitset; no float atomics.
- Code: occupancy uses `atomicOr` on `atomic<u32>`; owner/color are non-atomic but written by one workgroup per voxel.
- Status: aligned (assuming voxel ownership per tile/brick is exclusive, which it is).

### Determinism rules
- Spec: deterministic owner tie-break (min triangle index).
- Code: tracks `best = min(tri)` for each voxel; owner and color derived from best.
- Status: aligned.

### Buffer layout & bounds checks
- Spec: strict alignment; avoid out-of-bounds.
- Code: structured with `Params` and `CompactParams` on uniform buffers; occupancy bounds checked for expected words.
- Status: aligned, but see “Risk Areas” for size-dependent failures.

### WASM data exchange
- Spec: JSON + typed arrays; no renderer coupling.
- Code: WASM exports only data; rendering is in JS/Three.js.
- Status: aligned (renderer path removed).

## Risk Areas / Suspected Mismatches

1) **GPU compaction + render caps**
- `voxelize_triangles_positions` caps positions by `max_positions`.
- JS currently derives `max_positions` from device storage size or a hard cap (5,000,000).
- Risk: voxel count > max_positions silently truncates positions → partial render.

2) **Render chunking vs. voxel chunking**
- `render-chunk` affects only rendering (InstancedMesh batching).
- User expectations sometimes treat it as voxelization chunking.
- Risk: tuning render chunk doesn’t change voxelization; can look like missing voxels when it’s just rendering not finishing (time-sliced build).

3) **Time-sliced cube build**
- Async cube build yields partial on-screen geometry until all chunks are done.
- If parameters change frequently, build can be canceled and restarted; only the latest build completes.
- Risk: looks like “partial deterministic” output if build is repeatedly canceled or never reaches full completion.

4) **GPU fallback path visibility**
- GPU sparse voxelization can return empty occupancy (e.g., device limits, bad inputs, or validation errors).
- WASM fallback uses dense CPU voxelization; can be disabled if estimated memory exceeds limit.
- Risk: empty GPU results + fallback disabled means no voxels; logging shows chunk stats but not necessarily why the occupancy is empty.

5) **GPU compaction workgroup limits**
- `compact_sparse_positions_buffer` hard-errors if `brick_count > max_compute_workgroups_per_dimension`.
- Chunked path avoids this, but non-chunked compaction can fail at high brick counts.
- Risk: failures or truncations appear as missing voxels if not handled in JS.

## Open Questions for Stage 2 (Behavior vs. Intent)
- Are we hitting `max_positions` truncation when switching to cubes (points look fine)?
- Is the time-sliced cube build finishing, or being canceled by subsequent runs/UI updates?
- Are chunk sizes in voxelization capped too aggressively (`max_bricks_per_dispatch`) for high-res grids?
- Is `compact_sparse_positions` being used when GPU compact is off (should be JS expansion only)?

## Suggested Next Step
- Use this doc to annotate the *expected flow per mode* (points vs cubes, progressive vs non-progressive).
- Then compare with logs (voxel counts, chunk counts, render completion logs) to identify where the pipeline diverges.
