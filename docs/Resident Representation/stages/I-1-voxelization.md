# Stage I-1: Voxelization

**Type:** spec
**Status:** current
**Date:** 2026-03-31
**Stage type:** GPU compute (main thread) + CPU pre-pass (worker)
**Trigger:** New mesh load, procedural generation, or bulk edit.

> Transforms mesh triangles (positions + indices + materials) into GPU-resident chunk occupancy, palettes, and index buffers. The bridge between source geometry and the canonical chunk runtime.

---

## Purpose

Every chunk in the runtime begins life here. The voxelizer rasterizes input triangles into the 64x64x64 grid, writing `opaque_mask` bits for each occupied voxel and recording material palette entries from the source geometry.

No downstream consumer reads mesh triangles directly. All consumers read chunk occupancy and palette data written by I-1 (via I-2 upload). The voxelizer is the sole producer that converts continuous geometry into the discrete voxel truth.

---

## Design Principle

The pipeline's data model dictates the voxelizer's output interface. The voxelizer adapts to the pool's occupancy format, the material system's palette protocol, and the I-2 stage's upload contract. Where the legacy GPU voxelizer (`crates/voxelizer/`) uses a different internal format (linear bitfield, per-brick storage), the integration layer converts to what the pool expects. The algorithms (SAT overlap, CSR binning, workgroup-level triangle filtering) transfer; the buffer layouts and pipeline decomposition do not transfer blindly.

---

## Thread Boundary

Per ADR-0014 and [thread-boundary](../thread-boundary.md):

| Work | Thread | Why |
|---|---|---|
| OBJ parsing | Worker | CPU-only string processing |
| CSR spatial index construction | Worker | CPU-only, O(n_tri x avg_bricks); can be expensive for large meshes |
| Triangle data preparation (transform to grid space, compute AABB + plane) | Worker | CPU-only math, avoids blocking main thread |
| CSR + triangle data transfer | Worker → Main | Transferable ArrayBuffers, zero-copy |
| GPU buffer creation + upload (transient) | Main | GPU objects are main-thread-only |
| Voxelizer kernel dispatch | Main | GPU compute |
| Scatter-to-pool dispatch | Main | GPU compute |
| Material resolution dispatch | Main | GPU compute |
| Pool slot pre-allocation | Main (CPU) | Slot directory is CPU-managed |
| Stale signal (trigger I-3) | Main (CPU) | Sets `stale_summary` bits |

The worker produces CPU-side data structures. The main thread consumes them into GPU transient buffers, dispatches compute work, and writes results directly into the chunk pool. No GPU state crosses the worker boundary.

---

## Architecture: Three-Phase GPU Pipeline

The voxelization pipeline consists of three GPU compute phases dispatched sequentially within a single command encoder submission. All intermediate buffers are **transient** — allocated for this submission and destroyed afterward.

```
Phase 1 — Brick Rasterization
  Input:  CSR structure + prepared triangle data (from worker)
  Output: Per-brick occupancy bitfield + per-voxel owner triangle ID
  Method: SAT triangle-box overlap, one workgroup per brick

Phase 2 — Scatter to Pool
  Input:  Per-brick occupancy + owner + chunk-slot map
  Output: chunk_occupancy_atlas[slot] in column-major u64 format
  Method: One workgroup per brick, atomicOr into pool slots

Phase 3 — Material Resolution
  Input:  Per-brick owner_id + material_table + chunk-slot map
  Output: chunk_palette_buf[slot] + chunk_index_buf[slot] + palette_meta[slot]
  Method: One workgroup per chunk slot; scan occupied voxels, build palette, write index buffer
```

### Why three phases, not four

The legacy voxelizer uses four pipelines: rasterize, compact-positions, compact-attributes, compact-voxels. The compact pipelines exist to extract `CompactVoxel[]` for CPU readback. In the new architecture, there is no readback — the scatter phase writes directly to pool buffers. The compact pipelines are eliminated.

### Why not write pool format directly in Phase 1

The rasterizer kernel operates per-brick (spatial tiles for workgroup dispatch). Bricks and chunks are different spatial units — a brick may span chunk boundaries, and multiple bricks contribute to one chunk. The kernel's natural write granularity is per-brick. Phase 2 handles the brick→chunk scatter and format conversion.

An alternative would be to modify the rasterizer kernel to compute chunk-local coordinates and write column-major format directly. This merges Phases 1 and 2 but complicates the kernel with chunk boundary logic and changes the atomicOr target from per-brick buffers to the shared pool atlas. This is a valid optimization for later — the two-phase split is clearer for initial implementation and debugging.

---

## Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-1 | `triangle_data_buf` contains valid prepared triangles (6 vec4 per triangle: p0, p1, p2, aabb_min, aabb_max, plane) | Worker pre-pass |
| PRE-2 | `brick_origins_buf`, `brick_offsets_buf`, and `tri_indices_buf` contain a valid CSR structure mapping bricks to overlapping triangles | Worker pre-pass (CSR builder) |
| PRE-3 | `material_table` is populated with valid `MaterialEntry` structs for all MaterialIds that will be referenced | Scene init / material system |
| PRE-4 | `tri_material_table_buf` maps triangle index → MaterialId (packed u16x2 per u32) | Worker pre-pass (from OBJ `usemtl` groups) |
| PRE-5 | Grid parameters (voxel_size, world_origin, grid_dims) are set consistently with the chunk coordinate system | Scene config |
| PRE-6 | Pool slots have been pre-allocated for all chunks the mesh will occupy | Main thread, before dispatch |

---

## Inputs

### From Worker (CPU pre-pass)

| Data | Format | Construction |
|---|---|---|
| Prepared triangle array | `[vec4f; N_tri * 6]` — p0, p1, p2, aabb_min, aabb_max, plane | Transform to grid space, compute AABB with epsilon expansion, compute plane normal + d |
| Brick CSR: `brick_origins` | `[vec4u; N_bricks]` | `build_brick_csr()` — spatial index of which bricks overlap which triangles |
| Brick CSR: `brick_offsets` | `[u32; N_bricks + 1]` | CSR row pointers |
| Brick CSR: `tri_indices` | `[u32; N_entries]` | CSR column data (triangle indices per brick) |
| Triangle material table | `[u32; ceil(N_tri / 2)]` | Packed u16x2; maps triangle index → MaterialId |
| Grid spec | origin, voxel_size, dims, chunk_grid_dims | Determines chunk count and coordinate mapping |

All data arrives as Transferable ArrayBuffers via `postMessage`. The main thread uploads them to GPU transient storage buffers before dispatch.

### From Pool (pre-allocated)

| Buffer | Access | What's read |
|---|---|---|
| `chunk_occupancy_atlas` | Write (via Phase 2 atomicOr) | Target for scattered occupancy |
| `chunk_palette_buf` | Write (via Phase 3) | Target for palette entries |
| `chunk_index_buf` | Write (via Phase 3) | Target for bitpacked palette indices |
| `palette_meta` | Write (via Phase 3) | Target for palette_size + bits_per_entry |

### Slot Map (transient)

| Buffer | Format | What it does |
|---|---|---|
| `chunk_slot_map` | `array<u32>` — flat 3D grid indexed by chunk coord | Maps chunk grid position → pool slot index |

Constructed on CPU from the pre-allocated slot assignments. Uploaded to GPU transient buffer. Phases 2 and 3 use it to resolve which pool slot each voxel should be written to.

---

## Phase 1: Brick Rasterization

### Algorithm

For each brick in the CSR structure, one workgroup tests all candidate triangles against all voxels in the brick's region. This is the SAT (Separating Axis Theorem) conservative rasterization from the legacy `crates/voxelizer/`.

```
@compute @workgroup_size(WORKGROUP_SIZE, TILES_PER_WORKGROUP, 1)
fn rasterize(wg_id, lid):
  brick_index = wg_id.x * TILES_PER_WORKGROUP + lid.y
  brick_origin = brick_origins[brick_index].xyz

  // Thread 0: filter candidate triangles via plane-box test
  // Store active triangle indices in workgroup shared memory
  if lid.x == 0:
    for tri in CSR[brick_index]:
      if plane_box_intersects(tri.plane, brick_center, brick_half):
        active_tris[count++] = tri

  workgroupBarrier()

  // All threads: iterate voxels in brick, test against active triangles
  for voxel in brick (stride by WORKGROUP_SIZE):
    (gx, gy, gz) = brick_origin + local_voxel_offset
    center = vec3f(gx + 0.5, gy + 0.5, gz + 0.5)

    for tri in active_tris:
      if triangle_box_overlap(center, half, tri):
        // Mark occupied in per-brick bitfield
        atomicOr(&brick_occupancy[brick_word], 1u << bit)
        // Record owning triangle (min index wins)
        owner_id[brick_voxel_index] = min(owner, tri_index)
```

### Key algorithm properties (from legacy, preserved)

- **Conservative rasterization:** A voxel is marked occupied if any triangle intersects its AABB. Ensures watertight surfaces.
- **Epsilon expansion:** Triangle AABBs are expanded by a small epsilon during CSR construction to catch near-miss intersections.
- **Owner tracking:** Each voxel records the lowest-index triangle that overlaps it. This determines material assignment.
- **Overflow path:** If a brick has more than `MAX_ACTIVE_TRIS` (256) candidate triangles, the kernel falls back to direct CSR iteration (no shared memory filtering). This handles degenerate dense regions.

### Transient outputs

| Buffer | Format | Size |
|---|---|---|
| `brick_occupancy` | `array<atomic<u32>>` | `N_bricks * words_per_brick` u32 (1 bit per voxel, packed) |
| `owner_id` | `array<u32>` | `N_bricks * voxels_per_brick` u32 (one triangle index per voxel) |

These buffers are transient. They exist only for the duration of this command encoder submission.

---

## Phase 2: Scatter to Pool

### Problem

Phase 1 writes per-brick occupancy in a linear bitfield format. The pool expects per-chunk occupancy in column-major u64 format (`opaque_mask[x * 64 + z]` as two u32 words, bit `y`). Bricks and chunks are different spatial units — multiple bricks contribute to one chunk, and bricks may straddle chunk boundaries.

### Algorithm

One workgroup per brick. Each thread iterates over occupied voxels in the brick, computes their chunk-local coordinates, and atomicOr's the corresponding bit in the pool's occupancy atlas.

```
@compute @workgroup_size(64)
fn scatter_to_pool(wg_id, lid):
  brick_index = wg_id.x
  brick_origin = brick_origins[brick_index].xyz

  for voxel in brick (stride by 64):
    if !occupied(brick_occupancy, brick_index, voxel):
      continue

    // Global voxel position in grid space
    gx = brick_origin.x + local_x
    gy = brick_origin.y + local_y
    gz = brick_origin.z + local_z

    // Chunk coordinate (which chunk does this voxel belong to?)
    // CS = 62 (usable voxels per axis)
    chunk_cx = gx / CS
    chunk_cy = gy / CS
    chunk_cz = gz / CS

    // Local position within chunk (with +1 padding offset)
    lx = (gx % CS) + 1
    ly = (gy % CS) + 1
    lz = (gz % CS) + 1

    // Look up pool slot for this chunk
    slot = chunk_slot_map[chunk_cx, chunk_cy, chunk_cz]

    // Write to pool occupancy atlas in column-major u64 format
    slot_offset = slot * WORDS_PER_SLOT  // 8192
    col_idx = lx * CS_P + lz             // CS_P = 64
    word_idx = slot_offset + col_idx * 2 + (ly >> 5)
    bit = ly & 31
    atomicOr(&chunk_occupancy_atlas[word_idx], 1u << bit)
```

### Key design decisions

- **Euclidean division for negative coordinates:** If the grid origin can produce negative global voxel positions, `chunk_cx = floor_div(gx, CS)` and `lx = euclidean_mod(gx, CS) + 1`. This ensures correct chunk assignment regardless of world-space position.
- **Padding border:** Local coordinates are offset by +1 to account for the 1-voxel padding convention. Voxels land in the usable interior [1, 62].
- **Atomic writes to pool atlas:** Multiple bricks may contribute voxels to the same chunk. AtomicOr is safe because occupancy is monotonic (set bits, never clear).
- **Zero-init requirement:** Pool occupancy atlas slots must be zeroed before scatter. This is part of slot pre-allocation (before dispatch).

---

## Phase 3: Material Resolution

### Problem

After Phases 1-2, the pool's occupancy atlas is populated but palette and index buffers are empty. The `owner_id` scratch buffer records which triangle owns each voxel. The material system requires per-chunk palettes with global MaterialIds (see [material-system](../material-system.md)).

### Algorithm

One workgroup per chunk slot. Each thread iterates over the chunk's occupied voxels, resolves materials from the owner_id → material_table chain, and builds the palette + index buffer.

```
@compute @workgroup_size(256)
fn resolve_materials(wg_id, lid):
  slot = active_slots[wg_id.x]

  // Phase 3a: Scan occupied voxels, collect unique MaterialIds into palette
  // Uses workgroup shared memory for palette deduplication
  var palette: array<u16, MAX_PALETTE>  // shared
  var palette_size: u32 = 0             // shared atomic

  for voxel in chunk (stride by 256):
    if !occupied_in_pool(slot, x, y, z):
      continue

    // Reverse-map from pool local coords to grid global coords
    // to index into the owner_id scratch buffer
    tri_index = owner_id[reverse_map(slot, x, y, z)]
    mat_id = tri_material_table[tri_index]  // packed u16x2 lookup
    if mat_id == 0: mat_id = 1              // MATERIAL_EMPTY → MATERIAL_DEFAULT

    // Insert into palette (shared memory dedup)
    palette_idx = palette_insert(palette, palette_size, mat_id)

    // Write palette index into per-voxel index buffer
    // Bit width is determined after palette is complete (Phase 3b)
    temp_indices[voxel_linear_index] = palette_idx

  workgroupBarrier()

  // Phase 3b: Compute bits_per_entry from palette_size
  bits_per_entry = ceil_log2(palette_size)  // 1, 2, 4, 8, or 16

  // Phase 3c: Bitpack temp_indices into chunk_index_buf[slot]
  // Write palette to chunk_palette_buf[slot]
  // Write palette_meta[slot]
```

### Key design decisions

- **Palette dedup in shared memory:** Typical palette sizes are 1-16 entries. A linear scan in shared memory is sufficient and avoids complex data structures.
- **Two-sub-phase approach:** The bit width of index_buf entries depends on palette_size, which isn't known until all voxels are scanned. Phase 3a collects palette + raw indices, Phase 3b bitpacks.
- **Reverse mapping for owner_id lookup:** The scatter in Phase 2 writes to pool format, but owner_id is still in brick format. Phase 3 needs to map from (slot, local_x, local_y, local_z) back to the corresponding owner_id entry. This requires either:
  - (a) A parallel scatter of owner_id into a per-slot transient buffer during Phase 2, or
  - (b) A reverse index from chunk-local coords → brick index + brick-local offset

  Option (a) is simpler: extend Phase 2 to also scatter owner_id into a per-slot transient buffer alongside occupancy.

### Transient buffers consumed

| Buffer | From phase | Format |
|---|---|---|
| `owner_id` | Phase 1 | Per-brick, per-voxel triangle index |
| `owner_id_per_slot` | Phase 2 (extended) | Per-slot, per-voxel triangle index (scattered alongside occupancy) |
| `tri_material_table` | Worker | Packed u16x2 triangle → MaterialId |

### Pool buffers written

| Buffer | Per-slot size | What's written |
|---|---|---|
| `chunk_palette_buf[slot]` | Variable (max 64K x 2B packed) | Global MaterialId values, packed u16x2 per u32 |
| `chunk_index_buf[slot]` | Variable | Bitpacked per-voxel palette indices |
| `palette_meta[slot]` | 4 B | `palette_size` (u16) + `bits_per_entry` (u8) + reserved (u8) |

---

## Outputs

All durable outputs are written directly to the GPU chunk pool. No `CompactVoxel[]` courier is produced in the GPU path.

| Pool buffer | Written by | Per-slot size | Format |
|---|---|---|---|
| `chunk_occupancy_atlas[slot]` | Phase 2 | 32 KB (8192 x u32) | Column-major u64 bitpacked |
| `chunk_palette_buf[slot]` | Phase 3 | Variable | Packed u16x2 MaterialId array |
| `chunk_index_buf[slot]` | Phase 3 | Variable | Bitpacked per-voxel palette indices |
| `palette_meta[slot]` | Phase 3 | 4 B | palette_size + bits_per_entry |
| `chunk_coord[slot]` | Pre-dispatch (CPU) | 16 B | vec4i chunk coordinate |

### Transient buffers (discarded after submission)

| Buffer | Phase | Purpose |
|---|---|---|
| `triangle_data_buf` | 1 | Prepared triangles from worker |
| `brick_origins_buf` | 1, 2 | CSR brick origins |
| `brick_offsets_buf` | 1 | CSR row pointers |
| `tri_indices_buf` | 1 | CSR triangle lists |
| `brick_occupancy` | 1 → 2 | Per-brick bitfield (intermediate) |
| `owner_id` | 1 → 2 | Per-voxel triangle ownership (intermediate) |
| `owner_id_per_slot` | 2 → 3 | Per-slot scattered owner (intermediate) |
| `tri_material_table_buf` | 3 | Triangle → MaterialId mapping |
| `chunk_slot_map` | 2, 3 | Chunk grid → slot index mapping |

All transient buffers are created with `MAP_READ` disabled and are destroyed after the command encoder submission completes.

---

## Post-Dispatch (CPU, Main Thread)

After the GPU submission completes (signaled via `onSubmittedWorkDone` or next frame fence):

1. **Write `chunk_coord[slot]`** for each pre-allocated slot (if not already written during pre-allocation)
2. **Set `chunk_resident_flags[slot] = 1`** for all newly populated slots
3. **Set `stale_summary` bit** for each slot → triggers I-3 summary rebuild
4. **Release transient buffers** (or let them drop)

The CPU does not write `summary_rebuild_queue` directly. The GPU compaction pass (see [edit-protocol](../edit-protocol.md)) enqueues slots from `stale_summary` bits.

---

## Postconditions

| ID | Condition | Validates |
|---|---|---|
| POST-1 | Every triangle in the input that intersects a chunk's world-space extent has contributed occupancy bits to that chunk's pool slot | Completeness |
| POST-2 | Every occupied voxel bit in the pool corresponds to at least one intersecting input triangle | No phantom voxels |
| POST-3 | `chunk_palette_buf[slot]` contains exactly the set of MaterialIds referenced by occupied voxels in this slot | Palette completeness |
| POST-4 | `chunk_index_buf[slot]` entries are valid indices into `chunk_palette_buf[slot]` for all occupied voxels | Index validity |
| POST-5 | Padding border voxels (x, z = 0 or 63) are zero after voxelization (padding is populated by I-2 boundary copy, not by I-1) | Boundary correctness |
| POST-6 | `palette_meta[slot].palette_size` matches the number of entries in `chunk_palette_buf[slot]` | Metadata consistency |
| POST-7 | `palette_meta[slot].bits_per_entry` is the minimum power-of-two bit width sufficient for the palette size | Bit width correctness |
| POST-8 | Occupancy is written in column-major u64 format matching pool invariants OCC-1 through OCC-4 | Format correctness |
| POST-9 | All transient buffers are valid only for the duration of this submission; no downstream stage reads them | Transient lifetime |
| POST-10 | `stale_summary` bit is set for every slot written, ensuring I-3 will rebuild summaries | Summary trigger |

---

## Dispatch

### Pre-dispatch (CPU, main thread)

```
1. Receive CSR + triangle data from worker (ArrayBuffers)
2. Compute chunk grid dimensions from mesh AABB and voxel_size
3. Pre-allocate pool slots for all needed chunks (from free_slots)
4. Build chunk_slot_map (chunk grid index → slot)
5. Zero occupancy atlas regions for pre-allocated slots
6. Write chunk_coord[slot] for each slot
7. Create transient GPU buffers, upload worker data
```

### GPU dispatch (single command encoder)

```
Phase 1 — Brick rasterization:
  pipeline: voxelizer_pipeline
  workgroup_size: (WORKGROUP_SIZE, TILES_PER_WORKGROUP, 1)
  dispatch: ceil(N_bricks / TILES_PER_WORKGROUP) workgroups
  writes: brick_occupancy, owner_id

  barrier (compute → compute)

Phase 2 — Scatter to pool:
  pipeline: scatter_pipeline
  workgroup_size: (64, 1, 1)
  dispatch: N_bricks workgroups
  reads: brick_occupancy, owner_id, brick_origins, chunk_slot_map
  writes: chunk_occupancy_atlas (pool), owner_id_per_slot (transient)

  barrier (compute → compute)

Phase 3 — Material resolution:
  pipeline: material_resolve_pipeline
  workgroup_size: (256, 1, 1)
  dispatch: N_active_slots workgroups
  reads: chunk_occupancy_atlas (pool), owner_id_per_slot, tri_material_table
  writes: chunk_palette_buf (pool), chunk_index_buf (pool), palette_meta (pool)
```

### Post-dispatch (CPU, main thread)

```
8. Set chunk_resident_flags[slot] = 1 for all new slots
9. Set stale_summary bits → I-3 runs on next frame
10. Destroy transient buffers
```

---

## CompactVoxel Courier (CPU Fallback Path)

For CPU-originated data (procedural generation, network streaming, test scenes), the `CompactVoxel[]` courier format remains the interface into I-2:

```rust
#[repr(C)]
pub struct CompactVoxel {
    pub vx: i32,  // global voxel x
    pub vy: i32,  // global voxel y
    pub vz: i32,  // global voxel z
    pub material: u32,  // global MaterialId
}
```

The CPU path (procedural generators, `load_test_scene`) produces `CompactVoxel[]`, which I-2 consumes via `writeBuffer`. This path bypasses Phases 1-3 entirely — it is a separate entry point into the pool, not a fallback for the GPU voxelizer.

The GPU voxelizer does **not** produce `CompactVoxel[]`. It writes pool-format data directly. The two paths converge at the pool: after either path completes, the same pool slot state is expected (occupancy + palette + index_buf + palette_meta populated, stale_summary set).

---

## CPU Pre-Pass: CSR Spatial Index

The CSR (Compressed Sparse Row) spatial index maps bricks to overlapping triangles. It is constructed on the CPU worker thread before GPU dispatch.

### Algorithm (from `crates/voxelizer/src/csr.rs`)

```
Input: triangles (in grid space), brick_dim, grid_dims
Output: BrickTriangleCsr { brick_origins, brick_offsets, tri_indices }

1. For each triangle:
   - Compute AABB in grid space (with epsilon expansion)
   - Determine range of bricks overlapped: [min_brick, max_brick]
   - For each brick in range: increment brick's triangle count

2. Prefix sum over brick counts → brick_offsets

3. For each triangle (second pass):
   - Same brick range calculation
   - Scatter triangle index into tri_indices at correct offset
```

### Brick size selection

Brick size is a tuning parameter. Smaller bricks = tighter spatial filtering but more CSR overhead. Larger bricks = less overhead but more false positives in the rasterizer kernel.

| Brick dim | Voxels per brick | Notes |
|---|---|---|
| 8 | 512 | Good default — matches occupancy_summary bricklet size |
| 16 | 4096 | Fewer bricks, coarser filtering |
| 4 | 64 | Finer filtering, higher CSR overhead |

Default: **8** (aligns with the 8x8x8 occupancy_summary bricklets from I-3, keeping spatial granularity consistent across stages).

### Triangle preparation

Before CSR construction, triangles are transformed to grid space:

```
for each triangle (a, b, c) in world space:
  a_grid = (a - world_origin) / voxel_size
  b_grid = (b - world_origin) / voxel_size
  c_grid = (c - world_origin) / voxel_size

  aabb_min = min(a_grid, b_grid, c_grid) - epsilon
  aabb_max = max(a_grid, b_grid, c_grid) + epsilon

  normal = normalize(cross(b_grid - a_grid, c_grid - a_grid))
  d = -dot(normal, a_grid)

  emit: [a_grid, b_grid, c_grid, aabb_min, aabb_max, (normal, d)]
```

The 6-vec4 stride per triangle (positions + AABB + plane) matches the legacy `TRI_STRIDE = 6` convention. Pre-computing AABB and plane avoids redundant work in the GPU kernel.

---

## Legacy Algorithm Reference

The following algorithms from `crates/voxelizer/` transfer directly into the new pipeline:

| Algorithm | Source | Used in | Transfer status |
|---|---|---|---|
| SAT triangle-box overlap | `gpu/shaders.rs` `triangle_box_overlap()` | Phase 1 kernel | WGSL portable as-is |
| Plane-box intersection | `gpu/shaders.rs` `plane_box_intersects()` | Phase 1 kernel (tile filter + SAT early-out) | WGSL portable as-is |
| Workgroup triangle filtering | `gpu/shaders.rs` main() | Phase 1 kernel (shared memory active_tris) | WGSL portable as-is |
| CSR brick construction | `csr.rs` `build_brick_csr()` | CPU pre-pass | Rust portable as-is |
| CSR tile construction | `csr.rs` `build_tile_csr()` | CPU pre-pass (dense path) | Rust portable as-is |
| CPU SAT reference | `reference_cpu.rs` `triangle_box_overlap()` | Test oracle | Rust portable as-is |
| Hash color | `gpu/shaders.rs` `hash_color()` | Debug visualization only | Optional |

### What does NOT transfer

| Component | Why discarded |
|---|---|
| `GpuVoxelizer` struct (owns device/adapter) | New pipeline shares the Renderer's device |
| `compact_positions` / `compact_attrs` pipelines | No CPU readback needed; scatter writes to pool directly |
| `compact_voxels` pipeline | Replaced by Phase 3 material resolution writing pool format |
| Per-brick linear occupancy format | Pool uses column-major u64; Phase 2 converts |
| `VoxelizationOutput` / `SparseVoxelizationOutput` types | Output is pool-format, not intermediate structs |
| Buffer mapping / readback (`map_buffer_u32`) | No GPU→CPU transfer in the voxelization path |

---

## Multi-Chunk Handling

A mesh larger than 62 voxels on any axis spans multiple chunks. The pipeline handles this naturally:

1. **CPU pre-allocation:** From the mesh AABB and voxel_size, compute the chunk grid dimensions (`ceil(mesh_extent / (CS * voxel_size))` per axis). Allocate one pool slot per chunk.
2. **CSR covers the full grid:** Bricks tile the entire grid, not per-chunk. A brick near a chunk boundary contains voxels that may scatter to different chunks.
3. **Phase 2 scatter resolves chunk membership:** Each voxel's global position determines its chunk via integer division. The `chunk_slot_map` provides the slot index. Voxels from different chunks within the same brick are scattered to different pool slots.
4. **Phase 3 runs per slot:** Each slot's material resolution is independent.

### Voxel resolution selection

The user specifies a target resolution (voxels along the mesh's longest axis). The pipeline computes:

```
longest_axis = max(mesh_extent.x, mesh_extent.y, mesh_extent.z)
voxel_size = longest_axis / target_resolution
grid_dims = ceil(mesh_extent / voxel_size)
chunk_grid_dims = ceil(grid_dims / CS)  // CS = 62
```

Default target resolution: **62** (single chunk). Maximum practical: **~256** (4x4x4 = 64 chunks, 8M voxels).

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Single triangle:** One triangle in a known position produces occupied voxels along its surface. Verify column bits in pool format match expected positions.
2. **Axis-aligned quad:** A 4x4 quad at a known Y produces exactly 16 occupied voxels in a single Y-plane. Verify column-major format.
3. **Material assignment:** Two triangles with different materials produce a palette with two entries. Verify `index_buf` maps each voxel to the correct palette entry.
4. **Empty input:** Zero triangles produce zero occupancy and an empty palette.
5. **Palette compaction:** 1000 triangles sharing the same material produce a palette with one entry.
6. **Column-major format:** Verify `opaque_mask[x * 64 + z]` bit `y` is set for occupied voxel at local (x, y, z). Cross-reference with linear index.

### Property tests (Rust, randomized)

7. **Completeness:** For 100 random triangle soups, verify every triangle that intersects a voxel AABB produces an occupied bit at that position in the pool.
8. **Palette validity:** For random material assignments, verify every occupied voxel's `index_buf` entry resolves to a valid palette entry matching the source triangle's material.
9. **Chunk isolation:** Voxels from chunk A do not appear in chunk B's pool slot.
10. **Multi-chunk consistency:** A mesh spanning 2x2x2 chunks has correct occupancy in all 8 slots, with no gaps at chunk boundaries.
11. **Padding correctness:** Padding border voxels (x, z = 0 or 63) are zero in all slots (padding is I-2's responsibility, not I-1's).

### GPU validation

12. **CPU vs GPU agreement:** Run voxelization on both CPU reference and GPU pipeline for the same mesh. Readback GPU pool slot data and compare occupancy + palette against CPU reference.
13. **Atomic correctness:** Two overlapping triangles writing to the same column produce the union of their occupancy bits (no lost writes from atomicOr races).
14. **Scatter correctness:** Voxels near chunk boundaries land in the correct slot. Verify by checking pool slots against expected chunk coordinates.

### Cross-stage tests

15. **I-1 → I-3:** After I-1 populates pool slots, I-3 summary rebuild produces consistent flags (`is_empty=0` for non-empty chunks, correct AABB, correct `has_emissive`).
16. **I-1 → R-1:** After I-1 + I-3, the greedy mesher produces valid geometry from the pool occupancy. Visual confirmation of correct mesh shape.
17. **I-1 → R-5:** End-to-end: load OBJ → voxelize → summary → mesh → render. The rendered image is a recognizable voxelized version of the source mesh with correct material colors.

---

## See Also

- [pipeline-stages](../pipeline-stages.md) -- Stage I-1 buffer table and position in the ingest pipeline
- [chunk-contract](../chunk-contract.md) -- canonical chunk fields written by I-1 output
- [material-system](../material-system.md) -- MaterialEntry, palette protocol, material_table
- [thread-boundary](../thread-boundary.md) -- worker CPU / main thread GPU split
- [gpu-chunk-pool](../gpu-chunk-pool.md) -- slot allocation, atlas layout, pool buffer formats
- [I-2-chunk-upload](I-2-chunk-upload.md) -- CPU fallback path; consumes CompactVoxel[] for non-GPU data
- [I-3-summary-rebuild](I-3-summary-rebuild.md) -- consumes pool data populated by I-1; triggered by stale_summary
- [edit-protocol](../edit-protocol.md) -- stale_summary signaling; queue population
- [data/chunk-occupancy-atlas](../data/chunk-occupancy-atlas.md) -- occupancy layout invariants (OCC-1 through OCC-6)
