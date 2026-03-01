> **SUPERSEDED** — This document has been reorganized into `docs/voxelizer-integration/`.
> Content preserved in: `design/requirements.md`, `design/gpu-output-contract.md`, `design/cpu-ingestion.md`.
> This file is retained as a historical record. Do not implement from it.

---

# Voxelizer Chunk-Native Output: Design Requirements

Date: February 21, 2026
Status: Design — pre-implementation

---

## Guiding Principle

The greedy mesh chunk manager is the **canonical store for voxel space**. It
defines the contract. Data reaches it from any source — procedural noise,
manual edits, rasterized meshes — all through the same API: a set of
`(world_position, MaterialId)` writes. The chunk manager does not know or care
about the source.

The GPU voxelizer was built before the chunk manager existed. Its output format
was designed to support its own renderer, not to feed a downstream store. Now
that the chunk manager is the target, the voxelizer must adapt to meet it — not
the other way around.

This document defines requirements for a voxelizer output stage that produces
data in the form closest to what the chunk manager natively consumes, while
keeping the GPU doing the work it is fastest at and the CPU doing only what it
must.

---

## Design Goal

Produce from the voxelizer pipeline a compact list of occupied voxels with
materials already resolved, such that the CPU needs only to:

1. Group entries by chunk coordinate
2. Write each group into the chunk manager with a single palette-building pass

Everything else — identifying which voxels are occupied, resolving triangle
ownership, looking up the material for each voxel — should be completed on
the GPU before data crosses the bus.

---

## Current State and Its Costs

### What the GPU currently produces

`voxelize_surface_sparse` returns `SparseVoxelizationOutput`:
- `brick_origins: Vec<[u32; 3]>` — grid-space origins of occupied bricks
- `occupancy: Vec<u32>` — bitpacked, one bit per voxel across all bricks
- `owner_id: Vec<u32>` — dense, one `u32` per voxel slot (including unoccupied)
- `color_rgba: Vec<u32>` — dense debug coloring, same layout

`compact_sparse_attributes` exists as a second GPU pass and already produces a
compact list of only occupied voxels. Its current output per occupied voxel:
- `out_indices[i]` — flat grid-space linear index
  `gx + grid_dims.x * (gy + grid_dims.y * gz)`
- `out_owner[i]` — raw triangle index (owner_id)
- `out_color[i]` — RGBA debug hash

### CPU costs under the current plan

After GPU readback, the CPU must:

| Step | Cost | Notes |
|------|------|-------|
| Scan all occupancy bits | O(total_grid_voxels) | 16M iterations for 256³, most are zero |
| Resolve material per occupied voxel | O(occupied) | Array lookup, fast |
| Build intermediate Vec | O(occupied) | Allocation + copy |
| HashMap grouping by chunk | O(occupied) | One lookup per voxel |
| Palette writes per voxel | O(occupied × palette_ops) | Repack cascade on new materials |

The occupancy scan is the dominant unnecessary cost. Iterating 16M bits to
find 1-2M occupied voxels wastes CPU time proportional to grid volume, not
voxel fill. This is precisely what the GPU's compact pass already solves —
but the CPU translation layer currently ignores it and re-scans from scratch.

The palette repack is the other unpredictable cost: each new `MaterialId`
added to a chunk's palette triggers a 262K-slot repack. A mesh with 20 material
groups crossing 50 chunks could trigger hundreds of repacks on the first write.

---

## Proposed Design Split

### GPU is responsible for

1. **Identifying occupied voxels** — done via the existing compaction pass
2. **Resolving material** — looking up `material_table[owner_id]` on the GPU,
   producing `MaterialId` directly instead of a raw triangle index
3. **Computing global voxel coordinates** — converting grid-space
   `(gx, gy, gz)` to global chunk manager voxel space by adding `g_origin`
   (a new uniform)
4. **Outputting only occupied entries** — no zero-padding, no dense arrays

### CPU is responsible for

1. **Grouping by chunk coordinate** — computing `chunk = div_euclid(vx, CS)`
   and `local = rem_euclid(vx, CS)` per entry, grouping entries by chunk
2. **Building palette and writing voxels** — for each chunk's group, pre-scan
   unique materials, size the palette once, then write all voxels in one pass
   without per-voxel repack cascade
3. **Dirty marking** — after all chunks are written, mark touched chunks dirty
   with neighbor awareness

The CPU no longer needs to know about bricks, occupancy bits, or triangle
indices. It receives a flat list of `(global_vx, global_vy, global_vz,
material_id)` and ingests it into the chunk manager.

---

## GPU Output Contract

The new compact pass produces a single interleaved AoS buffer of length
`n_occupied` (one 16-byte struct per occupied voxel):

```wgsl
struct CompactVoxel {
    vx:       i32,   // global voxel X  (g_origin.x + gx)
    vy:       i32,   // global voxel Y  (g_origin.y + gy)
    vz:       i32,   // global voxel Z  (g_origin.z + gz)
    material: u32,   // MaterialId as u32 (value fits in u16, 1-based)
                     // 0xFFFFFFFF if owner was unresolved
}
```

`n_occupied` is returned via the existing atomic counter mechanism.
One readback copy brings the entire dataset across the bus.

**Why AoS over SoA:**
Single readback pass instead of four, and each voxel's data is spatially
co-located in the buffer — better cache behaviour during CPU grouping.
16 bytes per voxel (natural alignment, no padding waste).

**Why global voxel coords instead of grid-space:**
The chunk manager's `div_euclid` / `rem_euclid` grouping operates on global
signed integers. Computing `g_origin + grid_xyz` once on the GPU eliminates
that addition from every CPU iteration.

**Ordering:** unordered. The GPU atomic counter produces entries in
non-deterministic order. CPU performs the grouping.

---

## Required Changes to the GPU Pipeline

### New input buffer: `material_table`

```wgsl
@group(0) @binding(N) var<storage, read> material_table: array<u32>;
```

Passed by the caller (TypeScript via WASM). The application layer is the
authority for material assignment (see Guiding Principle).

**Packing:** two `u16` MaterialId values are packed per `u32` word to halve
buffer size. Triangle index `tri` maps to:

```wgsl
let word  = tri >> 1u;
let shift = (tri & 1u) << 4u;          // 0 or 16
let mat   = (material_table[word] >> shift) & 0xFFFFu;
```

The CPU packs the table before upload:
```rust
// pack pairs of u16 into u32
let packed: Vec<u32> = table.chunks(2).map(|pair| {
    (pair[0] as u32) | ((pair.get(1).copied().unwrap_or(0) as u32) << 16)
}).collect();
```

Shift/mask in the shader is negligible cost. Buffer is typically < 64 entries
for real-world OBJ files so the saving is proportionally large.

### New uniform field: `g_origin`

```wgsl
// Added to CompactAttrsParams:
g_origin: vec3<i32>,
_pad: u32,
```

Computed inside `voxelize_and_apply` (Rust), not by the TypeScript caller:

```rust
let g_origin = [
    (origin_world[0] / voxel_size).floor() as i32,
    (origin_world[1] / voxel_size).floor() as i32,
    (origin_world[2] / voxel_size).floor() as i32,
];
```

The TypeScript caller passes `origin_world` as before — the conversion is
an implementation detail of the Rust layer.

### Changes to the compact shader

The compact_attrs shader (`COMPACT_ATTRS_WGSL` in `crates/voxelizer/src/gpu/shaders.rs`)
currently produces `(linear_index, owner_id, color_rgba)`. It must be updated
to produce `(vx, vy, vz, material_id)` instead.

Key change within the per-voxel write block:

```wgsl
// Before
out_indices[idx] = linear_index;
out_owner[idx]   = owner_id[attr_index];
out_color[idx]   = color_rgba[attr_index];

// After
let raw_owner    = owner_id[attr_index];
let material_id  = select(
    0xFFFFFFFFu,                          // unresolved sentinel
    material_table[raw_owner],
    raw_owner != 0xFFFFFFFFu && raw_owner < arrayLength(&material_table)
);
out_vx[idx]       = i32(origin.x + vx) + params.g_origin.x;
out_vy[idx]       = i32(origin.y + vy) + params.g_origin.y;
out_vz[idx]       = i32(origin.z + vz) + params.g_origin.z;
out_material[idx] = material_id;
```

`out_color` is removed entirely — it was debug-only and has no role in chunk
manager ingestion.

### Existing pipeline infrastructure reused

The compaction pass already has:
- One workgroup per brick, 64 threads per workgroup striding through voxels
- Occupancy bit check per voxel
- Atomic counter for compact output
- Separate readback pass

All of this is unchanged. Only the input buffers and output data are modified.

---

## CPU Ingestion Requirements

The CPU receives `n_occupied` entries of `(vx, vy, vz, material_id)`.

### Step 1 — Group by chunk

```
chunks: HashMap<ChunkCoord, Vec<([u32;3], MaterialId)>>

for i in 0..n_occupied:
  vx, vy, vz = out_vx[i], out_vy[i], out_vz[i]
  if out_material[i] == 0xFFFFFFFF: use MATERIAL_DEFAULT
  else: material = out_material[i] as u16; if 0 → MATERIAL_DEFAULT

  coord = ChunkCoord { x: div_euclid(vx, CS), ... }
  local = [rem_euclid(vx, CS) as u32, ...]
  chunks[coord].push((local, material))
```

### Step 2 — Write each chunk

For each `(coord, entries)` pair:

```
1. get_or_create_chunk(coord)
2. For each entry: chunk.set_voxel_raw(lx, ly, lz, material)
   (no version increment yet — raw path)
3. chunk.increment_version()   (once for the whole batch)
```

Palette repack is handled by the existing `set_voxel_raw` + repack machinery.
Each new `MaterialId` encountered in a chunk may trigger a repack of 262K
palette slots. For typical OBJ files (< 20 material groups per chunk) this is
acceptable. If profiling later shows repack cost is measurable, a pre-scan of
unique materials per chunk to pre-size the palette before writes would
eliminate the cascade — but this is deferred.

### Step 3 — Dirty marking

After all chunks are written:

```
for coord in touched_chunks:
    mark_dirty(coord)
    for neighbor in coord.neighbors():
        if has_chunk(neighbor):
            mark_dirty(neighbor)  // boundary voxels may affect neighbor mesh
```

Dirty marking is deferred until all writes are complete to prevent greedy
merging on partial chunk state (spec §7.2).

---

## Constraints

| Constraint | Reason |
|------------|--------|
| Chunk manager internals unchanged | The canonical store defines its own format |
| GPU shader does not encode CS=62 | Chunk size is a CPU concern; GPU outputs global voxel ints |
| `material_table` supplied by application layer | Voxelizer has no knowledge of material semantics |
| Voxelizer core crate (`crates/voxelizer`) API surface unchanged | `wasm_voxelizer` reference implementation must continue to work |
| No `out_color` in new output | Debug color is irrelevant to chunk ingestion; remove to reduce bus traffic |
| `owner_id` sentinel `u32::MAX` handled before material lookup | GPU guard prevents out-of-bounds table access |

---

## What This Design Does Not Cover

- **Sorting the compact output by chunk on the GPU.** CPU grouping via HashMap
  is fast enough (O(n) with good constants). GPU radix sort would add
  significant shader complexity for marginal gain.
- **Writing `opaque_mask` directly from GPU.** Two reasons make this
  impractical. First, `opaque_mask` is `[u64; CS_P2]` and WGSL has no
  `atomic<u64>` — only `atomic<u32>`. Multiple threads writing different Y
  bits into the same column word simultaneously would race; splitting each u64
  into two u32 halves and issuing paired `atomicOr` calls is awkward and still
  fragile. Second, the shader would need to encode `CS_P = 64`, the
  `x * CS_P + z` column layout, and the `+1` padding offset — all
  chunk-internal details that create a hard dependency on the chunk format.
  Neither blocker is insurmountable, but together they make this not worth
  pursuing given the AoS compact output already eliminates the CPU occupancy
  scan, which was the dominant cost.
- **GPU-side palette building.** The palette is a dynamic variable-width
  structure. Building it on the GPU is not practical.
- **Streaming / chunked voxelization.** For grids too large for one GPU
  dispatch, the existing `voxelize_surface_sparse_chunked` mechanism handles
  this. The new compact pass applies to each chunk in the same way.

---

## Resolved Design Decisions

| # | Question | Decision | Rationale |
|---|----------|----------|-----------|
| 1 | Output buffer layout | **AoS** `{vx, vy, vz, material}[]` | Single readback pass; co-located data for CPU grouping |
| 2 | `g_origin` computation | **Inside `voxelize_and_apply`** (Rust) | Simpler caller API; conversion is a Rust implementation detail |
| 3 | Palette pre-sizing | **Rely on existing repack machinery** | Avoid premature optimisation; profile first |
| 4 | `material_table` packing | **Two `u16` per `u32`** with shift/mask | Memory conscious; shift/mask in shader is negligible cost |
