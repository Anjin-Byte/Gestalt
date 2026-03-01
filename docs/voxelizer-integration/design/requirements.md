# Integration Requirements

Date: February 22, 2026
Status: Authoritative

Consolidated from:
- `archive/voxelizer-chunk-native-output-design-requirements.md`
- `archive/voxelizer-materials-state-requirements-architecture-report.md` §3

---

## Scope

These requirements define what the voxelizer integration must satisfy. They are
implementation-neutral: they say *what*, not *how*. The `spec/` and `impl/`
documents define the how.

---

## GPU / CPU Division of Responsibility

| Responsibility | Owner | Rationale |
|----------------|-------|-----------|
| Identify occupied voxels | **GPU** | Compact pass already does this; CPU re-scan is waste |
| Resolve triangle → MaterialId | **GPU** | Eliminates per-voxel CPU lookup; done before bus transfer |
| Compute global voxel coordinates | **GPU** | Adds `g_origin` once per voxel; eliminates CPU coord reconstruction |
| Output only occupied entries | **GPU** | No padding, no dense arrays in compact output |
| Group by chunk coordinate | **CPU** | `div_euclid(CS=62)` is a CPU concern; GPU cannot encode CS |
| Write palette and voxel data | **CPU** | Palette is a dynamic variable-width structure; GPU impractical |
| Mark dirty chunks | **CPU** | Requires neighbor awareness and deferred sequencing |

---

## Correctness Requirements

**MAT-REQ-01 — Deterministic material assignment.**
Every occupied voxel receives exactly one MaterialId. The assignment is
deterministic: repeated voxelization of the same mesh with the same input produces
the same MaterialId for each voxel.

**MAT-REQ-02 — Deterministic tie-break.**
When multiple triangles intersect the same voxel, the voxel's material is
determined by the triangle with the minimum index in the index buffer. This is
the existing GPU behavior and must not be changed.

**MAT-REQ-03 — Consistency across chunk boundaries.**
A voxel near a chunk boundary must receive the same MaterialId regardless of which
chunk it is written into. The attribution is based on triangle provenance, not
spatial position relative to chunk boundaries.

**MAT-REQ-04 — Canonical empty material never written to solid voxel.**
`MATERIAL_EMPTY = 0` must never be written to an occupied voxel. Any path that
would produce `0` must fall through to `MATERIAL_DEFAULT = 1`.

---

## Data Contract Requirements

**MAT-REQ-05 — GPU output exposes MaterialId, not owner index.**
The compact pass output must carry resolved `u16 MaterialId` values, not raw
triangle indices. The CPU must not be required to perform the
`triangle_idx → MaterialId` lookup.

**MAT-REQ-06 — GPU output uses global signed integer voxel coordinates.**
Compact output must carry global chunk manager voxel coordinates (`i32`), computed
by the GPU as `g_origin + grid_xyz`. The CPU must not reconstruct coordinates from
brick origins and local offsets.

**MAT-REQ-07 — No float positions in the integration path.**
Voxel positions must not be represented as `[f32; 3]` world-space positions at
any point in the GPU→CPU→chunk manager path. Float authority for voxel writes
produces alignment ambiguity at chunk boundaries. Integer global voxel coordinates
are unambiguous.

**MAT-REQ-08 — Single GPU readback.**
All occupied voxel data (coordinates + materials) must cross the GPU-CPU bus in
a single readback. No separate readbacks for coordinates, owners, and colors.

---

## Performance Requirements

**MAT-REQ-09 — No CPU occupancy scan.**
The CPU must not iterate occupancy bits to find occupied voxels. The GPU compact
pass delivers only occupied voxels. CPU work scales with `n_occupied`, not with
`total_grid_voxels`.

**MAT-REQ-10 — Conversion overhead proportional to occupied voxels.**
Every per-voxel operation (grouping, coordinate lookup, palette write) must be
O(n_occupied), not O(grid_volume).

**MAT-REQ-11 — Minimal WASM boundary chatter.**
All GPU dispatch, conversion, and chunk ingestion happen within a single
`voxelize_and_apply` WASM call. No intermediate data structures cross the JS/WASM
boundary between the GPU dispatch and the chunk manager write.

**MAT-REQ-12 — Stable MaterialIds to reduce palette repacks.**
MaterialId values for the same material group must be the same across calls. The
caller builds the `material_table` from `buildMaterialTable()`, which produces
1-based sequential IDs from group index. IDs are stable as long as the group
ordering in the OBJ file is stable.

---

## Pipeline Integration Requirements

**MAT-REQ-13 — Chunk manager owns dirty marking and versioning.**
Dirty chunk marking, version increments, and neighbor sync remain in the chunk
manager's domain. The integration writes voxels via `set_voxel_raw` and
`increment_version`, then calls the chunk manager's dirty marking API. It does
not bypass or duplicate these mechanisms.

**MAT-REQ-14 — Legacy voxelizer path unchanged.**
`crates/wasm_voxelizer` and all its exports are not modified. The legacy sparse
preview path remains available for debug use. Only the chunk manager's WASM
binding gains the new `voxelize_and_apply` method.

---

## What This Design Does Not Cover

These items were considered and explicitly deferred. Do not implement them as
part of this integration.

**GPU-side sorting by chunk coordinate.**
CPU grouping via `HashMap<ChunkCoord, _>` is O(n_occupied) with good constants.
GPU radix sort would add significant shader complexity for marginal gain on
workload sizes typical for OBJ voxelization.

**Writing `opaque_mask` from the GPU.**
WGSL has no `atomic<u64>`. The chunk manager's `opaque_mask` is a `[u64; CS_P²]`
Y-column bitmask. Multiple GPU threads writing different Y-bits into the same
column word would race; splitting into u32 halves with paired `atomicOr` is fragile
and encodes chunk-internal constants (CS_P=64, padding offset) into the shader.
The compact output already eliminates the CPU occupancy scan, which was the
dominant cost. The `opaque_mask` is rebuilt by the CPU during the existing palette
write path.

**GPU-side palette building.**
The palette is a dynamic variable-width structure managed by the chunk manager.
Building it on the GPU is not practical.

**Streaming / chunked voxelization.**
For grids too large for one GPU dispatch, the existing `voxelize_surface_sparse_chunked`
mechanism handles batching. The new compact pass applies to each batch in the same
way. Streaming is not a new concern introduced by this integration.

**Pre-scanning unique materials to pre-size palette.**
Each new MaterialId added to a chunk's palette may trigger a repack of palette
slots. For typical OBJ files (< 20 distinct material groups per chunk) this repack
cost is acceptable. Pre-scanning unique materials per chunk before writes could
eliminate the cascade but is deferred until profiling shows measurable cost.
