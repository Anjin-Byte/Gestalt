**Type:** legacy
**Status:** legacy

> **SUPERSEDED** — This document describes Architecture A (CPU-side occupancy scan and material lookup).
> Architecture B (GPU-compact) is the current design. See `docs/voxelizer-integration/`.
> **Valuable content preserved in:**
> - Coordinate frame derivation → `spec/coordinate-frames.md`
> - Material pipeline (§2) → `spec/material-pipeline.md`
> - WASM API design (§3) → `spec/wasm-api.md`
> - Correctness invariants (§9.5, §10) → `spec/invariants.md`
> The implementation plan (§9.4 conversion algorithm, §4) is superseded by `impl/` docs.

---

# Voxelizer → Greedy-Native Integration: Architecture Specification

**Date:** 2026-02-20
Status: Authoritative specification
Audience: Voxelizer, WASM bindings, worker/runtime, and greedy-mesher maintainers

---

## Document Contract

1. **Type**: Prescriptive. This document resolves architectural ambiguities and fills
   gaps identified in the migration outline. It supersedes informal descriptions in
   upstream reports on every topic it covers.
2. **Relation to prior docs**:
   - Consumes: `voxelizer-greedy-native-migration-outline.md` (gaps addressed here)
   - Consumes: `voxelizer-greedy-mesher-unification-report.md` (problem framing)
   - Consumes: `voxelizer-materials-state-requirements-architecture-report.md`
   - Produced outputs feed: implementation of Phases 1–4 of the migration outline.
3. **Citation format**: Source references use the form `path/to/file.rs:line`.

---

## 1. Coordinate Frame Reconciliation

### 1.1 The Three Coordinate Spaces

Three distinct coordinate spaces are in play. They must be formally separated to
avoid confusion in any implementation.

**Space A — World Space (float)**

The common 3-D Euclidean space shared by the scene, the camera, and all objects.
Units are arbitrary engine units (e.g. metres). Positions are `Vec3` (f32).

**Space B — Voxelizer Grid Space (non-negative integer)**

The coordinate space internal to a single voxelization request. Defined by a
`VoxelGridSpec`:

```
struct VoxelGridSpec {
    origin_world: Vec3,   // World-space origin of grid voxel (0,0,0)
    voxel_size:   f32,    // Side length of each grid voxel in world units
    dims:         [u32; 3], // Grid extents: gx in [0, dims[0]), etc.
    world_to_grid: Option<Mat4>,
}
```

Source: `crates/voxelizer/src/core.rs:4–9`.

The forward mapping from World → Grid Space is:

```
world_to_grid_matrix() =
    scale(1 / voxel_size) * translate(-origin_world)
```

Source: `crates/voxelizer/src/core.rs:38–39`.

Applied to a world point `W`:

```
G = (W - origin_world) / voxel_size
```

Grid voxel `(gx, gy, gz)` occupies the unit cube `[gx, gx+1) × [gy, gy+1) ×
[gz, gz+1)` in Grid Space, which corresponds to the world-space box:

```
world_min = origin_world + (gx, gy, gz) * voxel_size
world_max = origin_world + (gx+1, gy+1, gz+1) * voxel_size
```

Grid coordinates are always non-negative (`u32`). The voxelizer clamps triangle
projections to `[0, dims[i])` during CSR construction
(`crates/voxelizer/src/csr.rs:190–197`).

**Space C — Greedy Global Voxel Space (signed integer)**

The coordinate space of the greedy chunk manager. Voxel addresses are signed
`[i32; 3]`, supporting negative coordinates for worlds that extend in all
directions. The chunk at chunk-coordinate `(cx, cy, cz)` contains the signed
global voxels:

```
vx in [cx * CS, (cx+1) * CS)
vy in [cy * CS, (cy+1) * CS)
vz in [cz * CS, (cz+1) * CS)
```

where `CS = 62` (usable voxels per chunk side).

Source: `crates/greedy_mesher/src/core.rs:16`.

The mapping from signed global voxel → chunk coordinate uses Euclidean division:

```
cx = div_euclid(vx, CS)
```

Source: `crates/greedy_mesher/src/chunk/coord.rs:85`.

The local coordinate within a chunk uses Euclidean remainder:

```
lx = rem_euclid(vx, CS),   lx in [0, CS)
```

Source: `crates/greedy_mesher/src/chunk/coord.rs:112`.

---

### 1.2 Formal Mapping: Grid Space → Greedy Global Voxel Space

**Claim**: Assuming the same `voxel_size` is used in both systems, the signed
global voxel index `V` corresponding to voxelizer grid voxel `G = (gx, gy, gz)` is:

```
V = G + G_origin

where G_origin = floor(origin_world / voxel_size)  (applied component-wise)
```

**Derivation**:

1. The world-space minimum corner of grid voxel `G` is:
   ```
   W_min = origin_world + G * voxel_size
   ```

2. In the greedy system, global voxel `V` occupies world-space box:
   ```
   [V * voxel_size,  (V+1) * voxel_size)
   ```
   (same `voxel_size` by assumption).

3. For the two voxels to be the same physical cube, their world-space minimum
   corners must coincide:
   ```
   V * voxel_size = origin_world + G * voxel_size
   V = origin_world / voxel_size + G
   ```

4. Since `V` must be an integer and `G` is already an integer:
   ```
   V = floor(origin_world / voxel_size) + G  =  G_origin + G
   ```

   The `floor` is needed only because floating-point representation of
   `origin_world / voxel_size` may be non-integer (see §1.3). When the alignment
   invariant holds, `floor` is a no-op.

**Implementation**: At conversion time, compute `G_origin` once from the
`VoxelGridSpec` that was used for the voxelization, then apply it to every brick
voxel. The conversion runs inside `crates/wasm_voxelizer`, where the `VoxelGridSpec`
is available.

```
// Pseudocode — runs once per voxelization session
G_origin: [i32; 3] = [
    floor(grid.origin_world.x / grid.voxel_size) as i32,
    floor(grid.origin_world.y / grid.voxel_size) as i32,
    floor(grid.origin_world.z / grid.voxel_size) as i32,
]

// For each occupied voxel (gx, gy, gz) in grid space:
vx = gx as i32 + G_origin[0]
vy = gy as i32 + G_origin[1]
vz = gz as i32 + G_origin[2]
```

---

### 1.3 Alignment Invariant

**Invariant VOX-ALIGN**: For each component `i ∈ {x, y, z}`:

```
origin_world[i] mod voxel_size == 0
```

i.e. `origin_world` is an exact multiple of `voxel_size` in every component.

**Consequence of violation**: If `origin_world[i] / voxel_size` has a fractional
part `ε ∈ (0,1)`, the world-space cube of grid voxel `(gx, gy, gz)` in component
`i` spans:

```
[ (floor(origin_world[i] / voxel_size) + gx + ε) * voxel_size,
  (floor(origin_world[i] / voxel_size) + gx + ε + 1) * voxel_size )
```

This box straddles two adjacent greedy voxels. The voxelizer assigns occupancy to
its grid voxel, but the greedy system's voxel boundary is offset by `ε`. Surfaces
near the boundary will be written into the wrong greedy voxel, producing seam
artifacts at chunk boundaries and incorrect mesh topology.

**Enforcement**:

The new `voxelize_triangles_chunk_deltas` WASM export (§3.4) must validate this
invariant before proceeding:

```rust
fn check_alignment(grid: &VoxelGridSpec) -> Result<(), String> {
    for i in 0..3 {
        let ratio = grid.origin_world[i] / grid.voxel_size;
        if (ratio - ratio.round()).abs() > 1e-4 {
            return Err(format!(
                "origin_world[{}]={} is not an exact multiple of voxel_size={}; \
                 chunk boundary alignment is not guaranteed",
                i, grid.origin_world[i], grid.voxel_size
            ));
        }
    }
    Ok(())
}
```

The caller (TypeScript layer) is responsible for constructing `VoxelGridSpec` with
an aligned origin. The common pattern is to snap to the nearest voxel grid point:

```
aligned_origin[i] = round(desired_origin[i] / voxel_size) * voxel_size
```

---

### 1.4 Shared Voxel Size Contract

**Invariant VOX-SIZE**: The `voxel_size` passed to the voxelizer must equal the
`voxel_size` configured in `WasmChunkManager` (via `RebuildConfig.voxel_size`,
`crates/wasm_greedy_mesher/src/lib.rs:455–458`).

This is a required precondition, not a checked invariant in the chunk manager itself.
The caller is responsible. The `voxelize_and_apply` WASM export (§3.2) validates
this constraint at call entry — before any GPU work begins — by comparing the
`voxel_size` argument against the value stored in `WasmChunkManager` at
initialization time. If they differ, the function rejects immediately with an
explanatory error string.

---

## 2. Material Pipeline

### 2.1 Current State Summary

The voxelizer stores per-voxel provenance, not semantic materials:

- `owner_id: Option<Vec<u32>>` — triangle index of the lowest-indexed triangle that
  intersects each voxel. Initialized to `u32::MAX`; written only for occupied voxels.
  Source: `crates/voxelizer/src/gpu/sparse.rs:279` (initialization),
  `crates/voxelizer/src/gpu/shaders.rs:205` (minimum-index selection),
  `crates/voxelizer/src/gpu/shaders.rs:255–256` (write path).

- `color_rgba: Option<Vec<u32>>` — debug color derived from `hash_color(owner_id)`.
  Source: `crates/voxelizer/src/gpu/shaders.rs:248`.

- `material_ids` is always `None` in active WASM entrypoints:
  `crates/wasm_voxelizer/src/lib.rs:205` (single dispatch),
  `crates/wasm_voxelizer/src/lib.rs:745` (chunked dispatch).

The greedy mesher's native type is `MaterialId = u16`
(`crates/greedy_mesher/src/core.rs:5`). The two reserved values are:
- `MATERIAL_EMPTY = 0` — absence of solid voxel
- `MATERIAL_DEFAULT = 1` — fallback solid material

Source: `crates/greedy_mesher/src/core.rs:8–10`.

---

### 2.2 Owner-ID Memory Layout in `SparseVoxelizationOutput`

For a sparse output with `N` bricks, `brick_dim = D`:

```
brick_voxels = D³

// owner_id flat array layout:
owner_id[b * brick_voxels + local_index]

where:
    b            = brick index in [0, N)
    local_index  = lx + D * (ly + D * lz)
    (lx, ly, lz) = local voxel coords within brick, each in [0, D)
```

Source: `crates/voxelizer/src/gpu/shaders.rs:251–256` (shader write path).

The `lx, ly, lz` values are derived from the linear invocation index `linear`:

```
lx = linear % D
ly = (linear / D) % D
lz = linear / (D * D)
```

Source: `crates/voxelizer/src/gpu/shaders.rs:181–183`.

**The global grid voxel** for brick `b` at local `(lx, ly, lz)` is:

```
(gx, gy, gz) = (brick_origins[b][0] + lx,
                brick_origins[b][1] + ly,
                brick_origins[b][2] + lz)
```

where `brick_origins[b]` is in Grid Space — i.e. the voxel-space coordinate of
the brick's first voxel within the voxelizer grid.

Source: `crates/voxelizer/src/csr.rs:214–217`:

```rust
brick_origins.push([x * brick_dim, y * brick_dim, z * brick_dim]);
```

Here `(x, y, z)` is the brick index (non-negative), so each origin component is
already expressed in grid voxels.

---

### 2.3 Material Table: Definition and Format

To convert `owner_id` (triangle index) → `MaterialId` (u16), the caller must
supply a per-triangle material assignment table at the WASM boundary.

**Definition**:

```
material_table: &[u16]

Invariant: material_table.len() == mesh.triangles.len()
material_table[tri_index] = MaterialId to assign to voxels whose owner is tri_index
```

A triangle with no assigned material must map to `MATERIAL_DEFAULT (1)`, never to
`MATERIAL_EMPTY (0)`. Using `0` would make occupied voxels invisible to the greedy
mesher.

**Format at WASM boundary**:

The table crosses the WASM boundary as a flat `&[u16]` parameter (zero-copy view
into JS `Uint16Array` via wasm_bindgen). The function signature is defined in §3.4.

**Responsibility assignment**:

The application layer (TypeScript, above the worker) is the authority for
the material table. It knows which mesh triangles belong to which material. The
table is constructed at mesh-load time and passed alongside the mesh geometry in
the voxelization request. It is not generated inside the voxelizer or WASM binding.

---

### 2.4 Tie-Break Policy (Deduplication)

When multiple triangles intersect the same voxel, the GPU shader selects the
triangle with the **minimum triangle index** as `owner_id`:

```wgsl
if (tri < best) { best = tri; }
```

Source: `crates/voxelizer/src/gpu/shaders.rs:205`.

This is the existing GPU behavior and must be preserved as the **canonical
tie-break policy**. The conversion from `owner_id → MaterialId` inherits this
policy automatically: whichever triangle has the lowest index wins, and its material
is used for the voxel.

**Implication**: The caller must order triangles such that lower-indexed triangles
have higher semantic priority when overlaps are expected. This is an application-
layer concern. The voxelizer and conversion code do not require changes.

**Edge case — unoccupied `owner_id` slot**:

If `owner_id == u32::MAX` for an occupied voxel (which should not occur in a
correct GPU run, but can occur in CPU fallback edge cases), the conversion must
map to `MATERIAL_DEFAULT`:

```
material = if owner_id == u32::MAX { MATERIAL_DEFAULT }
           else { material_table.get(owner_id as usize)
                  .copied().unwrap_or(MATERIAL_DEFAULT) }
```

---

### 2.5 WASM Input Interface: Material-Aware Entrypoint

The new WASM export adds two parameters to the existing chunked path: a flat
`Uint16Array` material table and a flag controlling whether the legacy owner/color
attributes are also stored.

The full signature is defined in §3.4. Here, the material-relevant inputs are:

```
material_table: Vec<u16>     // length == number of triangles
                              // material_table[i] = MaterialId for triangle i
```

The `store_owner` and `store_color` options remain but default `false` in the new
path. They can be set `true` for debug builds running dual-path verification.

---

### 2.6 Conversion: owner_id → MaterialId

Given a `SparseVoxelizationOutput` and a `material_table: &[u16]`:

```
for each brick b in 0..N:
    for lz in 0..D:
    for ly in 0..D:
    for lx in 0..D:
        local_index = lx + D * (ly + D * lz)
        occ_word   = b * words_per_brick + (local_index >> 5)
        occ_bit    = local_index & 31
        if (occupancy[occ_word] >> occ_bit) & 1 == 0: continue

        owner = owner_id[b * brick_voxels + local_index]
        material = if owner == u32::MAX || owner as usize >= material_table.len():
                       MATERIAL_DEFAULT
                   else:
                       material_table[owner as usize]
        if material == MATERIAL_EMPTY: material = MATERIAL_DEFAULT

        gx = brick_origins[b][0] + lx
        gy = brick_origins[b][1] + ly
        gz = brick_origins[b][2] + lz
        vx = gx as i32 + G_origin[0]
        vy = gy as i32 + G_origin[1]
        vz = gz as i32 + G_origin[2]

        // bucket by chunk
        cx = div_euclid(vx, CS)   // CS = 62
        lx_chunk = rem_euclid(vx, CS)
        // ...same for y, z
        emit(chunk_coord=(cx,cy,cz), local=(lx_chunk, ly_chunk, lz_chunk), material)
```

where:

```
words_per_brick = ceil(brick_voxels / 32) = (D³ + 31) / 32
brick_voxels    = D³
```

Source for occupancy layout: `crates/voxelizer/src/gpu/sparse.rs:154–155`.

---

### 2.7 u16 Range Adequacy

`MaterialId = u16` supports 65 536 distinct values, with 0 reserved for empty.
This gives 65 535 usable material slots. The current use case — per-object or
per-face material assignment on a single mesh — will not approach this limit.
If future use cases require more, the palette layer in `PaletteMaterials`
(`crates/greedy_mesher/src/chunk/palette_materials.rs:44–56`) still handles
compression; only the type width of `MaterialId` would need to change, which is
a single-line typedef change.

---

## 3. WASM API and Worker Protocol

### 3.1 Architectural Principle

The new voxelizer lives **inside** `wasm_greedy_mesher`. The JS worker calls a
single async function; all GPU work, conversion, and chunk ingestion happen inside
that one WASM call. No intermediate serialized format crosses the JS boundary. No
separate voxelizer worker is involved. The JS layer sees only typed-array inputs
and a stats output.

This eliminates:
- Cross-worker message overhead and all Transferable protocols
- Any intermediate binary buffer format
- The session ordering state machine previously required to coordinate two workers
- The risk of the two modules drifting out of sync

The legacy `wasm_voxelizer` module is left completely unchanged and continues to
serve its existing debug/preview use case.

---

### 3.2 WASM Export: `voxelize_and_apply`

Located in `crates/wasm_greedy_mesher/src/lib.rs`, added to `WasmChunkManager`.

```rust
/// GPU-voxelize a triangle mesh and apply the resulting voxels directly
/// into this chunk manager's storage.
///
/// All GPU dispatch, sparse-to-chunk conversion (§9), and chunk ingestion
/// happen inside this call. Dirty marking and rebuild scheduling are
/// performed after all GPU batches complete.
///
/// # Parameters
/// - `positions`:      flat f32 vertex positions [x0,y0,z0, ...]
/// - `indices`:        flat u32 triangle indices [i0,i1,i2, ...]
/// - `material_table`: u16 MaterialId per triangle; length == num_triangles
/// - `origin`:         [ox, oy, oz] — world-space grid origin (must satisfy VOX-ALIGN)
/// - `voxel_size`:     voxel side length; must equal this manager's voxel_size
/// - `dims`:           [dx, dy, dz] — grid extents in voxels
/// - `epsilon`:        triangle–voxel overlap tolerance (default 1e-4)
/// - `chunk_size`:     GPU dispatch batch size in bricks (0 = auto)
/// - `session_id`:     opaque u32 correlation ID returned in stats
///
/// # Returns Promise<WasmVoxelizeStats>
/// Resolves when all batches are complete and dirty marking is done. Rejects if:
/// - Voxelizer was not initialized (call `init_voxelizer()` first)
/// - `origin[i] mod voxel_size != 0` (VOX-ALIGN; see §1.3)
/// - `voxel_size` of this call != this manager's configured `voxel_size` (VOX-SIZE; see §1.4)
/// - `material_table.len()` != number of triangles
///
/// # Returned stats fields
/// - `session_id`:      echoes input session_id
/// - `chunks_touched`:  number of chunks that received at least one voxel
/// - `chunks_created`:  number of new chunks allocated
/// - `voxels_written`:  total occupied voxels written to chunk storage
/// - `bricks_processed`: total GPU brick dispatches across all batches
/// - `gpu_readback_ms`: wall-clock time for GPU dispatch + readback
#[wasm_bindgen]
pub fn voxelize_and_apply(
    &self,
    positions:      Vec<f32>,
    indices:        Vec<u32>,
    material_table: Vec<u16>,
    origin:         Vec<f32>,
    voxel_size:     f32,
    dims:           Vec<u32>,
    epsilon:        f32,
    chunk_size:     u32,
    session_id:     u32,
) -> js_sys::Promise { /* ... */ }
```

The function is `async` (returns a JS `Promise`) because it awaits GPU readback
for each batch, following the established pattern in `wasm_voxelizer`
(`crates/wasm_voxelizer/src/lib.rs:147–164`).

---

### 3.3 WASM Export: `init_voxelizer`

The GPU device must be initialized before `voxelize_and_apply` can be called.
This follows the same lifecycle as `WasmVoxelizer::new()`
(`crates/wasm_voxelizer/src/lib.rs:146–164`).

```rust
/// Initialize the integrated GPU voxelizer for this chunk manager.
///
/// Must be called once before any `voxelize_and_apply` call.
/// Obtains a wgpu Device/Queue by requesting the default WebGPU adapter.
///
/// # Returns Promise<void>
/// Rejects if no WebGPU adapter is available.
#[wasm_bindgen]
pub fn init_voxelizer(&mut self) -> js_sys::Promise { /* ... */ }
```

---

### 3.4 Internal Execution Flow

The body of `voxelize_and_apply` follows this sequence, entirely in Rust:

```
1. Validate inputs (alignment, voxel_size match, material_table length)
2. Build MeshInput from positions/indices
3. Build VoxelGridSpec from origin/voxel_size/dims
4. Compute G_origin = floor(origin / voxel_size)  (§1.2)
5. Build brick CSR (CPU-side, synchronous)
   — crates/voxelizer/src/csr.rs:160–236
6. Compute batch_count from CSR and chunk_size limit
   — crates/voxelizer/src/gpu/sparse.rs:82–96

For each batch b in 0..batch_count:
   6a. GPU dispatch (async) — run_sparse on sub-CSR for batch b
       — crates/voxelizer/src/gpu/sparse.rs:52–77
   6b. await GPU readback — SparseVoxelizationOutput for batch b
       — crates/voxelizer/src/gpu/sparse.rs:432–515
   6c. Convert: sparse bricks → chunk-local voxels (§9, synchronous)
       — for each occupied voxel: compute greedy coords, bucket by chunk
   6d. Ingest: apply bucket to ChunkManager via set_voxel_raw per voxel,
       single version increment per chunk  (§10)
       — crates/greedy_mesher/src/chunk/manager.rs:231–247 (existing pattern)
   6e. Accumulate touched ChunkCoords into session dirty set

7. Mark all chunks in dirty set dirty + schedule rebuilds
8. Return WasmVoxelizeStats
```

Steps 6c–6d run synchronously between GPU await points. In wasm_bindgen's async
model, the JS event loop is not available during synchronous Rust execution, so
the ChunkManager can be mutated safely without interior mutability between awaits.
For the `async fn` to hold the mutable `ChunkManager` reference across `await`
points, `WasmChunkManager::inner` is wrapped in `Rc<RefCell<ChunkManager>>` (see
§4.3). Each borrow is acquired immediately before and released immediately after
each synchronous Rust segment.

---

### 3.5 Worker Message Protocol

There is one worker for the new integrated path: the chunk manager worker.
The voxelizer is now a capability of that worker, not a separate entity.

**Request (application → worker)**:

```ts
interface VoxelizeAndApplyRequest {
    readonly type:          'cm-voxelize-and-apply';
    readonly sessionId:     number;
    readonly positions:     Float32Array;     // Transferable
    readonly indices:       Uint32Array;      // Transferable
    readonly materialTable: Uint16Array;      // Transferable
    readonly origin:        [number, number, number];
    readonly voxelSize:     number;
    readonly dims:          [number, number, number];
    readonly epsilon:       number;
    readonly chunkSize:     number;           // 0 = auto
}

// positions, indices, materialTable are listed in postMessage transfer array:
worker.postMessage(req, [req.positions.buffer, req.indices.buffer, req.materialTable.buffer]);
```

Transferring the three geometry arrays zero-copies them from the application thread
into the worker. No voxel data ever crosses a worker boundary in either direction.

**Response (worker → application)**:

```ts
interface VoxelizeAndApplyResult {
    readonly type:           'cm-voxelize-and-apply-result';
    readonly sessionId:      number;
    readonly chunksApplied:  number;
    readonly voxelsWritten:  number;
    readonly chunksCreated:  number;
    readonly bricksProcessed: number;
    readonly gpuReadbackMs:  number;
    readonly error:          string | null;
}
```

The existing chunk-update notifications (`last_swapped_coords`, `last_evicted_coords`)
are unaffected; they continue to fire on the next `update()` call after dirty chunks
are rebuilt.

---

## 4. Crate Architecture: Integrated Greedy Voxelizer

### 4.1 Decision

A new library crate `crates/greedy_voxelizer` is introduced. It bridges
`crates/voxelizer` (GPU compute) and `crates/greedy_mesher` (chunk storage).
`crates/wasm_greedy_mesher` acquires a dependency on it and gains GPU-related
imports. The existing `crates/voxelizer` and `crates/wasm_voxelizer` are not
modified.

### 4.2 Crate Dependency Graph (new)

```
crates/voxelizer                  (GPU compute core — unchanged)
    ↓ depends on
crates/greedy_voxelizer           (NEW: conversion + integrated driver)
    ↓ also depends on
crates/greedy_mesher              (chunk manager, palette — unchanged)
    ↓ consumed by
crates/wasm_greedy_mesher         (WASM bindings — extended)
```

`crates/wasm_voxelizer` remains a separate leaf depending only on
`crates/voxelizer`. It does not depend on `crates/greedy_voxelizer`.

### 4.3 `crates/greedy_voxelizer` — Public API

```rust
// crates/greedy_voxelizer/src/lib.rs

pub struct GreedyVoxelizer {
    gpu: GpuVoxelizer,   // from crates/voxelizer::gpu::GpuVoxelizer
}

impl GreedyVoxelizer {
    /// Create a new GreedyVoxelizer, requesting a WebGPU adapter.
    /// Matches the initialization pattern in wasm_voxelizer:
    ///   crates/wasm_voxelizer/src/lib.rs:149
    pub async fn new(config: GpuVoxelizerConfig) -> Result<Self, String>;

    /// Voxelize one mesh session and apply results directly to a ChunkManager.
    ///
    /// Processes bricks in GPU batches sequentially. After each batch's
    /// GPU readback, runs the sparse→chunk conversion (§9) and applies the
    /// result to `manager` via raw voxel writes. Dirty marking is deferred
    /// until all batches are complete.
    ///
    /// Returns accumulated session stats.
    pub async fn voxelize_into(
        &self,
        mesh:           &MeshInput,
        grid:           &VoxelGridSpec,
        material_table: &[u16],
        chunk_size:     usize,            // 0 = auto
        manager:        &mut ChunkManager,
    ) -> Result<VoxelizeIntoStats, String>;
}

pub struct VoxelizeIntoStats {
    pub bricks_processed: u32,
    pub voxels_written:   u32,
    pub chunks_touched:   u32,
    pub chunks_created:   u32,
}
```

`voxelize_into` is the single integration point. It encapsulates the full loop
from §3.4 steps 5–8 and has no WASM dependency; it is pure Rust and testable
without a browser environment (using `wgpu`'s Vulkan backend in native tests).

### 4.4 `crates/greedy_voxelizer` — Internal Modules

```
crates/greedy_voxelizer/
└── src/
    ├── lib.rs       — GreedyVoxelizer, VoxelizeIntoStats, re-exports
    └── convert.rs   — sparse_to_chunk_edits() implementing §9
```

`convert.rs` exports one function:

```rust
/// Convert one SparseVoxelizationOutput into chunk-grouped voxel writes
/// and apply them to `manager` via set_voxel_raw.
///
/// G_origin = floor(grid.origin_world / grid.voxel_size), precomputed
/// by the caller once per session.
///
/// Returns the set of ChunkCoords that received at least one write.
pub(crate) fn sparse_to_chunk_edits(
    output:         &SparseVoxelizationOutput,
    material_table: &[u16],
    g_origin:       [i32; 3],
    manager:        &mut ChunkManager,
) -> HashSet<ChunkCoord>;
```

### 4.5 `crates/wasm_greedy_mesher` Changes

**Cargo.toml additions**:

```toml
[dependencies]
greedy_voxelizer     = { path = "../greedy_voxelizer" }
wasm-bindgen-futures = "0.4"
wgpu                 = { version = "22", features = ["webgpu"] }
```

**`WasmChunkManager` struct change**:

Currently `inner: ChunkManager` is held directly. To allow the `voxelize_and_apply`
async block to mutate the manager across GPU await points, `inner` is changed to
`Rc<RefCell<ChunkManager>>`:

```rust
#[wasm_bindgen]
pub struct WasmChunkManager {
    inner:     Rc<RefCell<ChunkManager>>,
    voxelizer: Option<Rc<GreedyVoxelizer>>,
}
```

All existing synchronous methods (`set_voxel`, `set_voxels_batch`, `update`, etc.)
acquire and immediately release the `RefCell` borrow within their call. The
`voxelize_and_apply` async block clones the `Rc`s and acquires the `RefCell` borrow
in each synchronous segment between GPU awaits. Since WASM is single-threaded and
no JS event loop callbacks can run during a synchronous Rust segment, the borrow
is never contended at runtime. `RefCell` panics would only occur if the borrow
checker is bypassed by recursive JS callbacks during an await — which cannot happen
for the GPU readback awaits in this design.

### 4.6 What Is Not Changed

- `crates/voxelizer/` — unchanged
- `crates/wasm_voxelizer/` — unchanged
- `crates/greedy_mesher/` — unchanged
- The legacy `wasmVoxelizer` module and its preview path — unchanged
- All existing `WasmChunkManager` method signatures — unchanged (only `inner`
  field's wrapper type changes; existing callers see no difference)

---

## 5. Feature Flags

### 5.1 Scope and Kind

The feature flags are **runtime configuration values**, not compile-time features.
Compile-time flags would require rebuilding WASM to switch between paths, which is
unacceptable for staged rollout. The flags must be settable by the JS/worker layer
without a WASM rebuild.

### 5.2 Implementation Home

The flags live in the **chunk manager worker initialization message**, alongside
existing configuration (`max_chunks_per_frame`, `voxel_size`, etc.). They are
passed once at worker startup and held in worker-local state.

```ts
// chunkManagerTypes.ts — worker init message
interface ChunkManagerInitOptions {
    // ... existing fields ...
    voxelizerMode: 'legacy_preview' | 'greedy_native';
}
```

`legacy_preview` keeps the old behavior: the application calls `wasm_voxelizer`
separately for debug point-cloud output. `greedy_native` activates the new
integrated path via `voxelize_and_apply`.

### 5.3 Threading Through the System

There is one worker (the chunk manager worker) for the integrated path. The old
separate "voxelizer worker" is not involved.

```
Application layer (TypeScript)
    ├── reads voxelizerMode from runtime config (e.g. URL params, settings object)
    └── sends ChunkManagerInitOptions to chunk manager worker

Chunk manager worker (single worker, both mesh management and voxelization)
    ├── on startup: if voxelizerMode == 'greedy_native':
    │       await wasm_greedy_mesher.WasmChunkManager.init_voxelizer()
    │
    ├── on message 'cm-voxelize-and-apply':
    │       assert voxelizerMode == 'greedy_native'
    │       result = await wasmChunkManager.voxelize_and_apply(...)
    │       postMessage({ type: 'cm-voxelize-and-apply-result', ...result })
    │
    └── on message 'cm-set-voxels' (legacy):
            calls existing set_voxels_batch path

Legacy preview path (unchanged, separate from above)
    ├── application calls wasm_voxelizer.WasmVoxelizer.voxelize_triangles_chunked(...)
    └── JS adapter expands sparse output for point-cloud preview rendering
```

The flags are **not** threaded into Rust/WASM. Mode switching is expressed
entirely by which WASM function the worker calls, matching the existing pattern.

---

## 6. Partial Fill Policy

### 6.1 Problem Statement

The voxelizer grid covers `dims[0] × dims[1] × dims[2]` voxels, starting at
`origin_world`. This grid will generally not be aligned to chunk boundaries: a
grid of, say, 200 × 200 × 200 voxels starting at global voxel (10, 0, 0) will
touch chunks whose usable voxel range `[cx*62, (cx+1)*62)` extends beyond the
grid boundary. Voxels in those chunks that are outside the grid receive no data
from the voxelizer.

### 6.2 Formal Characterization

Let the voxelizer grid occupy global voxels:

```
vx in [G_origin[0],  G_origin[0] + dims[0])
vy in [G_origin[1],  G_origin[1] + dims[1])
vz in [G_origin[2],  G_origin[2] + dims[2])
```

A chunk `(cx, cy, cz)` is **partially covered** in axis `i` if its voxel range
`[ci*62, (ci+1)*62)` overlaps but does not contain the grid extent in that axis.
The voxels in the chunk outside the grid are not written by the voxelizer.

### 6.3 Policy Decision

**Policy VOX-PARTIAL**: The voxelizer delta applies only the voxels it has data for.
Voxels in a partially-covered chunk that fall outside the voxelizer grid are left
at their current state in the chunk manager. No explicit clearing or zeroing of
out-of-grid chunk voxels is performed.

**Rationale**:

1. The greedy chunk manager is a persistent mutable world store. A single
   voxelization session places a mesh into the world; it does not own or clear the
   surrounding space. Other edits (player edits, other mesh placements) may have
   previously written voxels in the same chunk.
2. If the application needs to clear a region before voxelizing (e.g. to replace
   an object), it must explicitly clear the affected chunk voxels via the existing
   `set_voxel_at` or a future `clear_voxel_region` API before issuing the delta.
3. Partial-fill chunks will still be marked dirty and rebuilt. The rebuilt mesh
   correctly includes all previously-set voxels in the chunk, not only those from
   the current voxelization.

**Corollary — empty chunks are not created**: If a chunk would receive zero voxels
from the voxelizer (i.e. it is entirely outside the grid), it is not created or
touched. `sparse_to_chunk_edits` (§4.4) only calls `set_voxel_raw` for voxels that
are both occupied in the sparse output and map to a valid chunk coordinate. Chunks
with zero intersecting voxels never appear in the `touched` set and receive no
`get_or_create_chunk` call.

---

## 7. GPU Batch Sequencing

### 7.1 Why Batching Exists

The GPU voxelizer uses `voxelize_surface_sparse_chunked`
(`crates/voxelizer/src/gpu/sparse.rs:30–50`) to process large meshes in multiple
bounded GPU dispatches. The batch count is determined before any GPU work begins,
from the brick CSR:

```rust
// crates/voxelizer/src/gpu/sparse.rs:42–49
let csr = build_brick_csr(mesh, grid, brick_dim, opts.epsilon);
let chunk_size = self.compute_chunk_size(brick_dim, opts, chunk_size,
                                        csr.brick_origins.len());
// batch_count = ceil(csr.brick_origins.len() / chunk_size)
```

Each batch produces one `SparseVoxelizationOutput` after GPU readback. In the
integrated design, batches are processed sequentially inside the single
`voxelize_and_apply` async call, entirely within Rust. No ordering protocol is
needed because there is no external consumer to sequence.

### 7.2 Sequential Batch Processing (In-Process)

The `process_chunks` loop in `crates/voxelizer/src/gpu/sparse.rs:98–120` already
processes batches sequentially — each `run_sparse` is `await`ed before the next
begins. `voxelize_into` in `crates/greedy_voxelizer` wraps this loop and extends
it with per-batch conversion and ingestion:

```
for batch b in 0..batch_count:
    output_b = await run_sparse(sub_csr_b)   // GPU readback for batch b
    touched_b = sparse_to_chunk_edits(       // synchronous conversion (§9)
                    &output_b, material_table, g_origin, &mut *manager.borrow_mut())
    touched.extend(touched_b)                // accumulate for final dirty marking

// After all batches:
for coord in touched:
    manager.borrow_mut().dirty_tracker.mark_dirty_with_neighbors(coord, ...)
```

**Dirty marking is always deferred to after the final batch.** This is correct by
construction: the `voxelize_into` function holds control until all batches are
processed. No intermediate dirty state is exposed to the rebuild scheduler.

**Proof that greedy merging cannot occur on partial state**: The `ChunkManager`
rebuild queue is only populated during `mark_dirty_with_neighbors`. This function
is only called after step (7) of §3.4, which runs after all GPU batches are
complete. The rebuild queue is drained by `ChunkManager::update()`, which is only
called by the worker's frame update loop. The frame update loop and `voxelize_and_apply`
are both on the same JS event loop thread and cannot interleave within a single
`await`-free segment.

### 7.3 Backpressure

The chunk manager worker's event loop is cooperative. While `voxelize_and_apply`
is executing (including between GPU await points), the worker cannot process other
messages. This provides natural backpressure: a second `cm-voxelize-and-apply`
message will not be processed until the current one resolves its Promise.

The application should not enqueue more voxelization requests than the user can
reasonably generate through interaction. For the current use case (user places
objects into the scene), inter-request spacing is at least hundreds of milliseconds,
which is far above the typical voxelization latency. No explicit queue depth limit
is needed at this time.

---

## 8. Phase 0 Baseline Metrics: Specification

The migration outline Phase 0 requires baseline metrics before any code changes.
The metrics must be machine-readable for regression detection.

### 8.1 What to Measure

Two paths produce metrics: the legacy path (application reconstructs world-space
voxel positions from the sparse output and calls `set_voxels_batch`) and the new
integrated path (`voxelize_and_apply`). The baseline run measures the legacy path;
the comparison run measures the integrated path on the same input mesh.

| Metric | Unit | Description |
|--------|------|-------------|
| `vox_gpu_time_ms` | ms | GPU dispatch time (from `DispatchStats.gpu_time_ms` when available) |
| `vox_readback_time_ms` | ms | Wall-clock from dispatch submit to GPU readback complete |
| `vox_convert_ms` | ms | Wall-clock for sparse→chunk conversion (§9) across all batches |
| `vox_occupied_voxels` | count | Total occupied voxels across all sparse batches |
| `vox_bricks_processed` | count | Total brick count across all sparse batches |
| `cm_chunks_dirtied` | count | Chunks marked dirty after session |
| `cm_rebuild_time_ms` | ms | Time to rebuild all dirtied chunks (from `FrameStats`) |
| `cm_voxels_written` | count | Voxels actually stored in chunk manager |
| `total_wall_ms` | ms | Wall-clock from `cm-voxelize-and-apply` postMessage to result received |

**Legacy-path-only metrics** (set to `null` in integrated path):

| Metric | Unit | Description |
|--------|------|-------------|
| `legacy_boundary_bytes` | bytes | Bytes transferred from application → `set_voxels_batch` (float32 positions) |
| `legacy_set_batch_ms` | ms | Time for `set_voxels_batch` call including internal HashMap grouping |

### 8.2 Storage Format

Metrics are emitted as a JSON object on the `performance` channel of the existing
logging system. No new infrastructure is required. Each voxelization session emits
one record:

```json
{
    "event": "vox_session_metrics",
    "session_id": 42,
    "path": "legacy_set_batch",
    "vox_gpu_time_ms": null,
    "vox_readback_time_ms": 38.2,
    "vox_convert_ms": null,
    "vox_occupied_voxels": 18432,
    "vox_bricks_processed": 128,
    "cm_chunks_dirtied": 14,
    "cm_rebuild_time_ms": 2.1,
    "cm_voxels_written": 18432,
    "total_wall_ms": 45.0,
    "legacy_boundary_bytes": 221184,
    "legacy_set_batch_ms": 1.4
}
```

After enabling `greedy_native`, the same record is emitted with `"path":
"greedy_native"`, `"vox_convert_ms"` populated, and both `"legacy_*"` fields
set to `null`. The primary comparison points are `total_wall_ms` (end-to-end
latency) and `cm_rebuild_time_ms` (quality of the resulting mesh, since partial
state during rebuild would produce broken geometry).

---

## 9. Corrected Conversion Algorithm

This section replaces §6 of the migration outline with a complete, unambiguous
algorithm.

### 9.1 Inputs

`sparse_to_chunk_edits` (in `crates/greedy_voxelizer/src/convert.rs`, §4.4)
receives one batch's GPU output and applies it. `G_origin` is computed once per
session by `voxelize_into` and passed to each call.

```
output:         &SparseVoxelizationOutput  // from one GPU batch readback
material_table: &[u16]                     // per-triangle MaterialId; len == num_triangles
g_origin:       [i32; 3]                   // floor(grid.origin_world / grid.voxel_size)
manager:        &mut ChunkManager          // target chunk store
```

There are no session sequencing parameters (`session_id`, `batch_index`,
`batch_count`). Sequencing is handled structurally by the `for` loop in
`voxelize_into` (§7.2); `sparse_to_chunk_edits` sees only one batch at a time.

### 9.2 Preconditions

```
assert output.owner_id.is_some()   // store_owner must have been true
assert check_alignment(grid).is_ok()   // VOX-ALIGN must hold
assert material_table.len() == num_triangles_from_indices
```

### 9.3 Precomputed Constants

```rust
let D:          u32  = output.brick_dim;
let brick_voxels: usize = (D * D * D) as usize;
let words_per_brick: usize = (brick_voxels + 31) / 32;
let G_origin: [i32; 3] = [
    (grid.origin_world.x / grid.voxel_size).floor() as i32,
    (grid.origin_world.y / grid.voxel_size).floor() as i32,
    (grid.origin_world.z / grid.voxel_size).floor() as i32,
];
let CS: i32 = 62;
```

Sources:
- `D` from `SparseVoxelizationOutput.brick_dim` (`crates/voxelizer/src/core.rs:144`)
- `words_per_brick` formula from `crates/voxelizer/src/gpu/sparse.rs:154–155`
- `G_origin` formula derived in §1.2
- `CS = 62` from `crates/greedy_mesher/src/core.rs:16`

### 9.4 Algorithm

```rust
// chunk_buckets: HashMap<[i32;3], Vec<(u8, u8, u8, u16)>>
//   key   = (cx, cy, cz)
//   value = list of (lx_chunk, ly_chunk, lz_chunk, material)

let owner_id = output.owner_id.as_ref().unwrap();
let occupancy = &output.occupancy;

for (b, brick_origin) in output.brick_origins.iter().enumerate() {
    for lz in 0..D {
    for ly in 0..D {
    for lx in 0..D {
        // 1. Check occupancy bit
        let local_index = (lx + D * (ly + D * lz)) as usize;
        let word_idx = b * words_per_brick + (local_index >> 5);
        let bit      = local_index & 31;
        if (occupancy[word_idx] >> bit) & 1 == 0 { continue; }

        // 2. Resolve material
        let owner = owner_id[b * brick_voxels + local_index];
        let material: u16 = if owner == u32::MAX
                               || owner as usize >= material_table.len() {
            MATERIAL_DEFAULT   // 1
        } else {
            let m = material_table[owner as usize];
            if m == MATERIAL_EMPTY { MATERIAL_DEFAULT } else { m }
        };

        // 3. Grid-space voxel (non-negative, within this voxelizer grid)
        let gx = brick_origin[0] + lx;
        let gy = brick_origin[1] + ly;
        let gz = brick_origin[2] + lz;

        // 4. Greedy global voxel (signed)
        let vx: i32 = gx as i32 + G_origin[0];
        let vy: i32 = gy as i32 + G_origin[1];
        let vz: i32 = gz as i32 + G_origin[2];

        // 5. Chunk coordinate
        let cx = vx.div_euclid(CS);
        let cy = vy.div_euclid(CS);
        let cz = vz.div_euclid(CS);

        // 6. Local coordinate within chunk, range [0, 62)
        let lx_c = vx.rem_euclid(CS) as u8;
        let ly_c = vy.rem_euclid(CS) as u8;
        let lz_c = vz.rem_euclid(CS) as u8;

        // 7. Apply directly to chunk manager
        let chunk = manager.get_or_create_chunk(ChunkCoord::new(cx, cy, cz));
        chunk.set_voxel_raw(lx_c as u32, ly_c as u32, lz_c as u32, material);
        touched.insert(ChunkCoord::new(cx, cy, cz));
    }}}
}

// Return the set of touched ChunkCoords for deferred dirty marking in §7.2
touched
```

### 9.5 Correctness Invariants

**Invariant C1 — Occupancy conservation**: The number of voxels emitted equals the
number of occupied bits in `output.occupancy`. In debug builds, assert:

```
total_emitted == output.occupancy.iter().map(|w| w.count_ones()).sum::<u32>()
```

**Invariant C2 — Local coordinate range**: For all emitted `(lx_c, ly_c, lz_c)`:

```
lx_c in [0, 62)  ∧  ly_c in [0, 62)  ∧  lz_c in [0, 62)
```

This follows from `rem_euclid(CS)` with `CS = 62`, which always returns a value
in `[0, 62)` regardless of the sign of the dividend.

**Invariant C3 — Material validity**: No emitted material is `MATERIAL_EMPTY (0)`.
Ensured by the `if m == MATERIAL_EMPTY { MATERIAL_DEFAULT }` guard in step 2.

**Invariant C4 — Chunk coordinate consistency**: For all emitted `(cx, cy, cz)` and
their local `(lx_c, ly_c, lz_c)`, the round-trip holds:

```
div_euclid(cx * CS + lx_c, CS) == cx
rem_euclid(cx * CS + lx_c, CS) == lx_c
```

This is a standard property of Euclidean division and is tested in
`crates/greedy_mesher/src/chunk/coord.rs:201–236`.

---

## 10. `sparse_to_chunk_edits`: Behavior Specification

### 10.1 Role in the System

`sparse_to_chunk_edits` (in `crates/greedy_voxelizer/src/convert.rs`, §4.4) is a
**pure Rust, synchronous, internal function**. It is not a WASM export. It runs
between GPU await points inside `voxelize_into` (§4.3), with exclusive mutable
access to `ChunkManager` via the `RefCell` borrow (§4.5).

Its relationship to the existing `set_voxels_batch`
(`crates/greedy_mesher/src/chunk/manager.rs:214–248`) is one of equivalent
semantics with a different calling convention:

| Aspect | `set_voxels_batch` | `sparse_to_chunk_edits` |
|--------|--------------------|-------------------------|
| Input grouping | Groups edits by chunk internally using a `HashMap` | Receives occupancy bits and resolves chunk coords inline |
| Per-chunk write | `set_voxel_raw` for each voxel | Same |
| Version increment | Once per chunk per call | Once per chunk per call (see §10.4) |
| Dirty marking | Immediately, inside the call | Deferred — returns `HashSet<ChunkCoord>` for later (§10.2) |
| Caller | TypeScript worker, via WASM boundary | `voxelize_into`, in-process Rust |

The allocation of a `HashMap` inside `set_voxels_batch` is replaced by the
iteration structure of `sparse_to_chunk_edits` itself, which processes voxels
grouped by brick. Bricks within a single chunk are contiguous in the CSR ordering
(`crates/voxelizer/src/csr.rs:218`: sorted by `(bz, by, bx)`), so cache locality
of `set_voxel_raw` writes is comparable.

### 10.2 Deferred Dirty Marking

`sparse_to_chunk_edits` does **not** call `mark_dirty_with_neighbors`. It returns
a `HashSet<ChunkCoord>` of every chunk that received at least one `set_voxel_raw`
write. The caller — the `for` loop in `voxelize_into` (§7.2) — accumulates this
set across all batches, then calls `mark_dirty_with_neighbors` for each coord
after the final batch.

This design means:

1. No chunk is enqueued for rebuild while another batch's voxels are still in
   flight. Greedy merging always sees the complete set of changes from a session.
2. `sparse_to_chunk_edits` is stateless with respect to the session; it has no
   session ID, no batch index, and no knowledge of whether more batches follow.
   All session-level bookkeeping is in `voxelize_into`.

There are no `apply_chunk_deltas_deferred` or `flush_session_dirty` WASM exports.
Those concepts do not exist in the integrated architecture.

### 10.3 Padding Offset Correctness

`Chunk::set_voxel_raw(x, y, z, material)` internally calls:

```rust
self.voxels.set(x as usize + 1, y as usize + 1, z as usize + 1, material);
```

Source: `crates/greedy_mesher/src/chunk/chunk.rs:149–153`.

The local coordinates `(lx_c, ly_c, lz_c)` emitted by the conversion (§9.4) are
in the range `[0, 62)`, matching the `x, y, z` parameter range of `set_voxel_raw`
(which guards `x >= Self::SIZE` = `x >= 62`). The `+1` padding offset is applied
inside `set_voxel_raw`, not by the caller. The conversion does not need to apply
any padding offset.

### 10.4 Version Semantics

`sparse_to_chunk_edits` calls `increment_version()` once per chunk it touches,
regardless of how many voxels in that chunk were written. This matches the existing
`set_voxels_batch` pattern (`crates/greedy_mesher/src/chunk/manager.rs:243`):

```rust
chunk.increment_version();
```

For a single-batch session, each touched chunk receives exactly one version
increment. For multi-batch sessions, a chunk touched by `k` different GPU batches
accumulates `k` version increments across those calls. This is functionally correct:
the rebuild scheduler responds only to `mark_dirty_with_neighbors` (called once
after the final batch), not to version changes during a session. The version counter
tracks edit events, not voxel counts, so multiple increments within a session are
semantically harmless.

See Open Question 3 (§11) for discussion of whether per-batch version increments
warrant a future policy change.

---

## 11. Open Questions (Deferred to Implementation)

These questions do not block the Phase 1–2 implementation but must be resolved
before Phase 3 (production rollout):

1. **Session ID generator**: The `session_id` field in `VoxelizeAndApplyRequest`
   (§3.5) and returned in the result is an opaque u32 correlation ID. The
   application layer is the natural generator, since it initiates voxelization
   requests and correlates results to UI actions. The chunk manager worker echoes
   the ID from the request into the result without modification. This requires no
   implementation decision inside Rust/WASM; it is solely an application-layer
   convention.

2. **Material table lifetime**: The material table is passed per-voxelization call.
   For repeated voxelization of the same mesh with the same materials, this is
   redundant — the table is transferred across the worker boundary on every call
   as a `Transferable`. A future optimization could register a material table by
   handle in the worker and reference it by ID in subsequent calls. Defer this;
   the immediate contract is per-call, and for typical use (one voxelization per
   user interaction), the transfer cost is negligible.

3. **Version increment batching in `sparse_to_chunk_edits`**: The current
   specification (§10.4) calls `increment_version()` once per chunk per
   `sparse_to_chunk_edits` call — i.e. once per GPU batch per touched chunk. For
   meshes requiring many batches, a chunk touched by multiple batches accumulates
   multiple version increments. This is functionally correct (the rebuild scheduler
   only reacts to `mark_dirty_with_neighbors`, not to version changes during a
   session). However, if version semantics are ever used for external change
   tracking (e.g. streaming updates to a server), the per-batch version semantics
   should be revisited. For now, increment once per batch is the simplest correct
   policy.

---

## Appendix A: Key Source Locations Quick Reference

| Symbol | File | Line |
|--------|------|------|
| `VoxelGridSpec` | `crates/voxelizer/src/core.rs` | 4 |
| `world_to_grid_matrix()` | `crates/voxelizer/src/core.rs` | 34–40 |
| `SparseVoxelizationOutput` | `crates/voxelizer/src/core.rs` | 143 |
| `MeshInput.material_ids` | `crates/voxelizer/src/core.rs` | 88 |
| `brick_origins` construction | `crates/voxelizer/src/csr.rs` | 214–217 |
| `words_per_brick` formula | `crates/voxelizer/src/gpu/sparse.rs` | 154–155 |
| `owner_id` initialization | `crates/voxelizer/src/gpu/sparse.rs` | 279 |
| `owner_id` write (sparse) | `crates/voxelizer/src/gpu/shaders.rs` | 255–256 |
| `owner_id` minimum selection | `crates/voxelizer/src/gpu/shaders.rs` | 205 |
| `occupancy` write (sparse) | `crates/voxelizer/src/gpu/shaders.rs` | 251–254 |
| Local index formula | `crates/voxelizer/src/gpu/shaders.rs` | 181–183 |
| `material_ids: None` (single) | `crates/wasm_voxelizer/src/lib.rs` | 205 |
| `material_ids: None` (chunked) | `crates/wasm_voxelizer/src/lib.rs` | 745 |
| `MaterialId = u16` | `crates/greedy_mesher/src/core.rs` | 5 |
| `MATERIAL_EMPTY`, `MATERIAL_DEFAULT` | `crates/greedy_mesher/src/core.rs` | 8–10 |
| `CS_P = 64`, `CS = 62` | `crates/greedy_mesher/src/core.rs` | 14–16 |
| `ChunkCoord::from_voxel` | `crates/greedy_mesher/src/chunk/coord.rs` | 82–89 |
| `ChunkCoord::voxel_to_local` | `crates/greedy_mesher/src/chunk/coord.rs` | 109–116 |
| `Chunk::set_voxel_raw` (+1 pad) | `crates/greedy_mesher/src/chunk/chunk.rs` | 149–153 |
| `ChunkManager::set_voxels_batch` | `crates/greedy_mesher/src/chunk/manager.rs` | 214–248 |
| `WasmChunkManager::set_voxels_batch` | `crates/wasm_greedy_mesher/src/lib.rs` | 486–492 |
| `PaletteMaterials` | `crates/greedy_mesher/src/chunk/palette_materials.rs` | 44–56 |
| `voxelize_surface_sparse_chunked` | `crates/voxelizer/src/gpu/sparse.rs` | 30–50 |
| `process_chunks` sequential loop | `crates/voxelizer/src/gpu/sparse.rs` | 98–120 |
