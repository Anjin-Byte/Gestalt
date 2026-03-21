# Chunk Field Registry

Explicit classification of every field on a runtime chunk.
This is the precise contract. [[chunk-contract]] is the narrative explanation.

---

## Classification Axes

| Axis | Meaning |
|---|---|
| **Authoritative** | Ground truth. No consumer may override. Producers write here only. |
| **Derived** | Computed from authoritative fields. May be discarded and rebuilt at any time. |
| **Rebuildable** | Can be recomputed deterministically from authoritative fields alone, with no external input. |
| **GPU-resident** | Has a live copy in GPU memory during rendering. |
| **CPU-mirrored** | CPU copy exists alongside the GPU copy and stays in sync. |
| **Required: traversal** | Needed by A&W, GI raymarching, probe queries, or world-space ray work. |
| **Required: meshing** | Needed by the greedy mesher to produce chunk geometry. |
| **Required: culling** | Needed by frustum cull, Hi-Z test, or indirect arg generation. |

`NOW` = current implementation. `TARGET` = GPU-resident architecture target.

---

## Field Matrix

### `opaque_mask : [u64; 4096]`

Binary voxel occupancy. Column-major: `opaque_mask[x*64 + z]` is a u64 of Y-axis bits.
The usable interior is indices 1–62 on each axis (1-voxel padding border).

| Axis | Value | Notes |
|---|---|---|
| Authoritative | **YES** | The occupancy data itself. Nothing derives it. |
| Derived | NO | |
| Rebuildable | NO | This is the data. It can only be re-ingested, not recomputed. |
| GPU-resident | NOW: NO · TARGET: **YES** | `r32uint` 3D texture or `array<u32>` storage buffer per chunk slot |
| CPU-mirrored | NOW: only copy · TARGET: **YES** | CPU copy stays authoritative for edits; GPU copy is synchronized from it |
| Required: traversal | **YES** | Per-voxel bit test in A&W inner loop; GI raymarch occlusion test |
| Required: meshing | **YES** | Face culling bitwise pass reads columns; greedy merge reads face masks |
| Required: culling | indirect | Culling uses `aabb` and `chunk_flags`, which derive from this |

---

### `materials.palette : Vec<MaterialId>`

Ordered list of unique material IDs referenced by this chunk.
Palette size determines index bit width: 2 entries → 1 bit, 4 → 2 bit, 256 → 8 bit.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | **YES** | Material assignment is ground truth. |
| Derived | NO | |
| Rebuildable | NO | Material assignment originates from the producer (voxelizer / edit). Cannot be reconstructed from occupancy alone. |
| GPU-resident | NOW: NO · TARGET: **YES** | `array<u32>` per chunk slot — material IDs indexed by palette entry |
| CPU-mirrored | NOW: only copy · TARGET: **YES** | Same pattern as `opaque_mask` |
| Required: traversal | **PARTIAL** | Emissive material lookup in GI; `has_emissive` flag derived from palette |
| Required: meshing | **YES** | Per-quad material ID assigned from palette during greedy merge |
| Required: culling | NO | |

---

### `materials.index_buf : Vec<u64>` (bitpacked)

Per-voxel indices into the palette. Packed at minimum bit width for current palette size.
Automatically repacked when palette grows past current bit capacity.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | **YES** | Per-voxel material assignment. |
| Derived | NO | |
| Rebuildable | NO | Same as palette — assignment is producer-originated. |
| GPU-resident | NOW: NO · TARGET: **YES** | `array<u32>` per chunk slot, same packing as CPU |
| CPU-mirrored | NOW: only copy · TARGET: **YES** | |
| Required: traversal | **PARTIAL** | Required for material queries (emissive value at hit point) |
| Required: meshing | **YES** | Per-voxel material resolved during face expansion |
| Required: culling | NO | |

---

### `coord : ChunkCoord { x: i32, y: i32, z: i32 }`

Chunk identity in chunk-space. World origin = `coord * CS * voxel_size` where CS = 62.
Euclidean division handles negative coordinates correctly.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | **YES** | Identity. Cannot be derived. |
| Derived | NO | |
| Rebuildable | NO | Identity. |
| GPU-resident | NOW: NO · TARGET: **YES** | Push constant or uniform per draw call; also stored in `chunk_slot_table_gpu` for traversal coord→slot lookup |
| CPU-mirrored | YES | CPU is the slot table authority |
| Required: traversal | **YES** | World-space position reconstruction; chunk DDA level uses coord to compute world bounds |
| Required: meshing | **YES** | Vertex world-space offset applied during mesh expansion |
| Required: culling | **YES** | AABB world position derived from coord |

---

### `data_version : u64`

Monotonic counter. Incremented on every voxel write. Resets on chunk eviction/recreation.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | **YES** | Metadata, but authoritative — it records edit history. |
| Derived | NO | |
| Rebuildable | NO | Monotonic; past value is gone after eviction. |
| GPU-resident | NO | CPU-only. Used only for async mesh conflict detection. |
| CPU-mirrored | N/A | CPU-only. |
| Required: traversal | NO | |
| Required: meshing | **YES** | Captured at mesh job start; compared on swap to detect stale results. |
| Required: culling | NO | |

---

### `state : ChunkState`

Lifecycle state: `Dirty | Meshing { data_version } | ReadyToSwap { data_version } | Clean`.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | NO | |
| Derived | **YES** | Computed from edit events and async job lifecycle. |
| Rebuildable | **YES** | Resets to `Dirty` on reload; meshing re-runs. No data is lost. |
| GPU-resident | NO | CPU-only. |
| CPU-mirrored | N/A | CPU-only. |
| Required: traversal | NO | Traversal reads committed occupancy only. State is irrelevant to ray queries. |
| Required: meshing | **YES** | Controls whether meshing runs, whether results are swapped in. |
| Required: culling | NO | |

---

### `mesh : Option<ChunkMesh>`

Greedy-meshed geometry: positions, normals, indices, UVs, material IDs.
Currently stored CPU-side; uploaded to Three.js `BufferGeometry` on demand.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | NO | |
| Derived | **YES** | Produced by greedy mesher from `opaque_mask` + `materials`. |
| Rebuildable | **YES** | Fully deterministic from authoritative fields. Discard at any time. |
| GPU-resident | NOW: YES (Three.js) · TARGET: **YES** (vertex/index pool) | GPU copy is the only copy in target architecture |
| CPU-mirrored | NOW: YES · TARGET: **NO** | CPU copy discarded after GPU upload in target |
| Required: traversal | NO | Traversal uses occupancy, not mesh triangles. |
| Required: meshing | N/A | This is the meshing output. |
| Required: culling | **YES** | Vertex/index ranges consumed by indirect draw; AABB derived from occupancy (not mesh) |

---

### `pending_mesh : Option<ChunkMesh>`

Mesh result from an in-flight async job, waiting for version check before swap.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | NO | |
| Derived | **YES** | Same derivation as `mesh`. Transient. |
| Rebuildable | **YES** | |
| GPU-resident | NO | Held CPU-side until version check passes, then uploaded. |
| CPU-mirrored | N/A | Transient. Discarded on version mismatch or after swap. |
| Required: traversal | NO | |
| Required: meshing | **YES** | Async swap mechanism. Without this, version-conflicted meshes would corrupt the scene. |
| Required: culling | NO | |

---

## Planned Fields (not yet implemented)

These fields do not exist in the current codebase. They are specified here because future consumers require them and their classification must be decided before implementation begins.

---

### `occupancy_summary : [u32; N]` — coarse bricklet occupancy grid

Divides the 62³ usable chunk volume into an 8³ grid of ~8³ bricklets (512 bricklets total).
One bit per bricklet: 1 if any voxel in that bricklet is occupied, 0 if entirely empty.
512 bits = 16 u32 words per chunk.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | NO | |
| Derived | **YES** | Computed from `opaque_mask` by OR-ing all voxel bits within each bricklet region. |
| Rebuildable | **YES** | One pass over `opaque_mask`. Fast: ~4096 u64 OR reductions. |
| GPU-resident | TARGET: **YES** | `array<u32>` flat buffer, 16 words per chunk slot. |
| CPU-mirrored | TARGET: **NO** | Built on GPU after occupancy upload; no CPU copy needed. |
| Required: traversal | **YES** | Level 1 in three-level DDA: skip empty bricklets before descending to voxel test. |
| Required: meshing | NO | Mesher operates on full `opaque_mask` directly. |
| Required: culling | NO | Culling uses `aabb` and `chunk_flags`. |

---

### `chunk_flags : u32` — packed summary bits

| Bit | Name | Derivation |
|---|---|---|
| 0 | `is_empty` | `opaque_mask` is all zero |
| 1 | `is_fully_opaque` | `opaque_mask` is all one |
| 2 | `has_emissive` | any entry in `materials.palette` has emissive > 0 |
| 3–31 | reserved | |

Note: `is_resident` is **not** a bit in `chunk_flags`. It lives in the separate `chunk_resident_flags` buffer, written by the CPU slot director on load/evict. See [[edit-protocol]] — Residency State section.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | NO | |
| Derived | **YES** | Each bit derived from `opaque_mask` or `materials.palette`. |
| Rebuildable | **YES** | Single pass over `opaque_mask` + palette scan. Trivially fast. |
| GPU-resident | TARGET: **YES** | Flat `array<u32>` indexed by slot. One u32 per chunk. |
| CPU-mirrored | TARGET: **NO** | All bits computed GPU-side by summary rebuild pass. `is_resident` has moved to `chunk_resident_flags`. |
| Required: traversal | **YES** | `is_empty` → skip entire chunk in chunk-level DDA. `has_emissive` → include in GI traversal. |
| Required: meshing | **PARTIAL** | `is_empty` can skip meshing entirely for empty chunks. |
| Required: culling | **YES** | `is_empty` and `is_resident` used in cull pass before AABB test. |

---

### `aabb : (vec3<f32>, vec3<f32>)` — tight world-space occupancy bounds

World-space min/max of the actual occupied voxels within the chunk.
Tighter than the full chunk boundary, especially for sparse chunks.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | NO | |
| Derived | **YES** | Scan `opaque_mask` for occupied range on each axis; apply `voxel_size` and `coord` offset. |
| Rebuildable | **YES** | Deterministic from `opaque_mask` + `coord` + `voxel_size`. |
| GPU-resident | TARGET: **YES** | `array<vec4f>` pairs (min+padding, max+padding), indexed by slot. |
| CPU-mirrored | TARGET: **NO** | Computed on GPU after occupancy upload. |
| Required: traversal | NO | Chunk DDA uses coord + fixed chunk size. AABB is tighter but not required. |
| Required: meshing | NO | Mesher uses full occupancy, not tight bounds. |
| Required: culling | **YES** | Primary input to Hi-Z AABB test. Tighter bounds = fewer false culls. |

---

### `palette_meta : array<u32, N_SLOTS>` — per-slot palette descriptor ★

One u32 per slot. Bits 0–15: `palette_size` (u16). Bits 16–23: `bits_per_entry` (u8). Bits 24–31: reserved.
`bits_per_entry` is authoritative — written by CPU at chunk load and palette resize; never derived at runtime in shaders.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | **YES** | CPU-written at chunk load and palette resize. |
| Derived | NO | |
| Rebuildable | NO | Depends on producer-assigned palette size; not derivable from occupancy alone. |
| GPU-resident | TARGET: **YES** | Flat `array<u32>` indexed by slot. |
| CPU-mirrored | TARGET: **YES** | CPU holds `palette_size` and `bits_per_entry` to write the field on load. |
| Required: traversal | **PARTIAL** | R-6 index unpack: `bits_per_entry` required to decompose `chunk_index_buf` at a hit voxel. |
| Required: meshing | **YES** | Mesher resolves palette index per voxel using `bits_per_entry`. |
| Required: culling | NO | |

---

### `material_table : array<MaterialEntry, MAX_MATERIALS>` — scene-global material properties ★

Flat array indexed by `MaterialId` (u16). `MaterialEntry` = 16 bytes (4 × packed f16 pairs):
albedo RGB, roughness, emissive RGB, opacity. Scene-scoped, not per-chunk.
`MAX_MATERIALS = 4096` → 64KB, fits in GPU L2 cache.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | **YES** | Material properties are ground truth. Written by CPU on material registration or property change. |
| Derived | NO | |
| Rebuildable | NO | Producer-originated. |
| GPU-resident | TARGET: **YES** | Single shared storage buffer bound as read-only in R-5, R-6, I-3. |
| CPU-mirrored | TARGET: **YES** | CPU holds property values to write on registration or change. |
| Required: traversal | **PARTIAL** | R-6: emissive property fetch after DDA confirms hit. Never part of the DDA inner loop. |
| Required: meshing | NO | Greedy mesher emits global `MaterialId` per quad; it does not read `MaterialEntry` properties. |
| Required: culling | NO | |

---

### `meshlet_desc_pool : array<MeshletDesc>` — per-meshlet surface cluster descriptors ★

Flat pool of all meshlets across all chunk slots. Each entry holds a world-space conservative
AABB, index range into `meshlet_index_pool`, vertex base into `vertex_pool`, parent chunk
slot, and `built_from_version`. See [[meshlets]] for full field layout.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | NO | |
| Derived | **YES** | Built from chunk surface mesh (itself derived from occupancy + materials). |
| Rebuildable | **YES** | Deterministic from mesh output; full chunk re-cluster on any mesh change. |
| GPU-resident | TARGET: **YES** | Flat array; region per chunk allocated from a freelist. |
| CPU-mirrored | TARGET: **NO** | Built and consumed GPU-side. |
| Required: traversal | NO | Product 3 only; Product 1 traversal reads `chunk_occupancy_atlas`. |
| Required: meshing | NO | |
| Required: culling | **YES** | R-4 phase 2 meshlet AABB test. |

---

### `meshlet_range_table : array<MeshletRange, N_SLOTS>` — per-slot index into meshlet_desc_pool ★

One `{start: u32, count: u32}` per slot. Written by swap pass after meshlet build completes.
`count = 0` means no meshlets built yet for this slot; R-4 falls back to chunk-level draw.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | NO | |
| Derived | **YES** | Written by swap pass after verifying `built_from_version`. |
| Rebuildable | **YES** | Re-populated on next meshlet build. |
| GPU-resident | TARGET: **YES** | Fixed N-slot flat array; always allocated. |
| CPU-mirrored | TARGET: **NO** | |
| Required: traversal | NO | |
| Required: meshing | NO | |
| Required: culling | **YES** | R-4 phase 2 dispatch per surviving chunk. |

---

### `meshlet_version : u32` per slot — version stamp of last meshlet build ★

`chunk_version[slot]` value at the time the meshlet build pass last committed valid meshlets
for this slot. R-4 phase 2 checks `meshlet_version[slot] == chunk_version[slot]` before
iterating meshlets; mismatch triggers chunk-level fallback draw.

| Axis | Value | Notes |
|---|---|---|
| Authoritative | NO | |
| Derived | **YES** | Control-plane tag; stamped by meshlet build pass. |
| Rebuildable | **YES** | Re-stamped on next successful build. |
| GPU-resident | TARGET: **YES** | Flat `array<u32>` indexed by slot. |
| CPU-mirrored | TARGET: **NO** | |
| Required: traversal | NO | |
| Required: meshing | NO | |
| Required: culling | **YES** | Freshness gate in R-4 phase 2. |

---

## Summary Table

| Field | Auth | Derived | Rebuild | GPU (target) | CPU-mirror | Traversal | Meshing | Culling |
|---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| `opaque_mask` | ✓ | | ✗ | ✓ | ✓ | ✓ | ✓ | indirect |
| `materials.palette` | ✓ | | ✗ | ✓ | ✓ | partial | ✓ | |
| `materials.index_buf` | ✓ | | ✗ | ✓ | ✓ | partial | ✓ | |
| `coord` | ✓ | | ✗ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `data_version` | ✓ | | ✗ | | | | ✓ | |
| `state` | | ✓ | ✓ | | | | ✓ | |
| `mesh` | | ✓ | ✓ | ✓ | ✗ | | output | ✓ |
| `pending_mesh` | | ✓ | ✓ | | | | ✓ | |
| `occupancy_summary` ★ | | ✓ | ✓ | ✓ | ✗ | ✓ | | |
| `chunk_flags` ★ | | ✓ | ✓ | ✓ | ✗ | ✓ | partial | ✓ |
| `aabb` ★ | | ✓ | ✓ | ✓ | ✗ | | | ✓ |
| `palette_meta` ★ | ✓ | | ✗ | ✓ | ✓ | partial | ✓ | |
| `material_table` ★ | ✓ | | ✗ | ✓ | ✓ | partial | | |
| `meshlet_desc_pool` ★ | | ✓ | ✓ | ✓ | ✗ | | | ✓ |
| `meshlet_range_table` ★ | | ✓ | ✓ | ✓ | ✗ | | | ✓ |
| `meshlet_version` ★ | | ✓ | ✓ | ✓ | ✗ | | | ✓ |

★ = planned, not yet implemented

---

## Hard Rules Derived from This Table

**Rule 1 — Authoritative fields are never recomputed.**
`opaque_mask`, `materials.palette`, `materials.index_buf`, and `coord` are the only fields that cannot be reconstructed. They must never be evicted without a plan to re-populate them from a producer.

**Rule 2 — All derived fields are safe to discard.**
Every non-authoritative field can be dropped and rebuilt from the authoritative set. Caches, meshes, summaries, and flags are all expendable.

**Rule 3 — Traversal must not read `mesh`.**
`mesh` is a raster surface structure. Traversal queries must read `opaque_mask` and `occupancy_summary`. Reading triangle geometry for ray work conflates Product 1 and Product 2. See [[layer-model]].

**Rule 4 — Culling output must not filter traversal input.**
`chunk_resident_flags.is_resident` and camera-visibility results from the cull pass must not gate which chunks are queried by A&W or GI. A chunk invisible to the camera may still be hit by a light ray.

**Rule 5 — `data_version` is the async correctness invariant.**
Any system that reads authoritative fields asynchronously must capture `data_version` at read time and validate it before committing results. Mismatched version → discard and re-run.

**Rule 6 — GPU copies are synchronized from CPU, not the reverse.**
Until GPU-side editing is explicitly implemented, CPU is the write authority for authoritative fields. GPU copies are updated after CPU writes, not before. Reading GPU copies as authoritative is an error.

---

## See Also

- [[chunk-contract]] — narrative explanation with edit semantics and residency protocol
- [[layer-model]] — three-product architecture; why traversal and culling must not share filters
- [[traversal-acceleration]] — three-level DDA consuming `opaque_mask`, `occupancy_summary`, `chunk_flags`
- [[gpu-chunk-pool]] — slot allocation, atlas layout, CPU→GPU sync implementation
