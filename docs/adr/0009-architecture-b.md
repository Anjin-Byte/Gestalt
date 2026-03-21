# ADR-0009: GPU-Compact Voxelizer Integration (Architecture B)

Date: February 22, 2026
Status: **Accepted**
Supersedes: `docs/greedy-meshing-docs/adr/0005-voxelizer-to-mesher-integration.md`

---

## Context

The GPU voxelizer (`crates/voxelizer`) produces `SparseVoxelizationOutput`:
brick-grouped sparse data with bitpacked occupancy, dense per-voxel triangle
indices (`owner_id`), and debug colors. Converting this to chunk manager writes
requires finding occupied voxels and resolving their materials.

Two architectures were considered.

### Architecture A — CPU-side conversion (prior approach)

The GPU outputs `SparseVoxelizationOutput` unchanged. A CPU-side function
`sparse_to_chunk_edits` then:

1. Iterates every voxel slot in every brick (O(total_grid_voxels) — up to 16M
   iterations for a 256³ grid, most of which are empty)
2. Checks each occupancy bit to find occupied voxels
3. Looks up `material_table[owner_id]` for each occupied voxel
4. Computes world-space coordinates from `brick_origin + local_xyz`
5. Groups results by chunk coordinate via HashMap
6. Calls `set_voxels_batch` on the chunk manager

Architecture A requires no GPU shader changes. All logic is in a new Rust crate.

**Identified as the prior approach in:**
- `archive/voxelizer-greedy-integration-implementation-plan.md`
- `archive/voxelizer-greedy-native-migration-outline.md`
- `archive/voxelizer-greedy-integration-spec.md` (§9 conversion algorithm)

### Architecture B — GPU-compact integration (this decision)

The GPU compact pass is extended to:

1. Accept a `material_table` buffer (packed u16 pairs)
2. Accept a `g_origin` uniform (global voxel offset)
3. Resolve `MaterialId = material_table[owner_id]` on the GPU
4. Output global voxel coordinates `(g_origin + grid_xyz)`
5. Output only occupied voxels as AoS `CompactVoxel[] {vx, vy, vz, material}`

The CPU then:

1. Groups `CompactVoxel` entries by `div_euclid(vx, CS)` chunk coordinate
2. Calls `set_voxel_raw` + `increment_version` per chunk
3. Marks dirty chunks and neighbors after all writes

The CPU occupancy scan does not exist. Material lookup does not happen on the CPU.

---

## Decision

**Architecture B.**

---

## Rationale

### The occupancy scan is the dominant unnecessary cost

The GPU compact pass (`compact_sparse_attributes` in
`crates/voxelizer/src/gpu/compact_attrs.rs`) already scans the occupancy bitfield
and outputs only occupied voxels. Architecture A re-scans those same bits on the
CPU — iterating O(grid_volume) entries to recover what the GPU already computed.
For a 256³ grid with 10% fill, this is 16M bit checks to produce 1.6M results.
The GPU had already done the scan.

### Material resolution belongs on the GPU

Each occupied voxel requires a `material_table[owner_id]` lookup. This is an array
index — negligible cost per voxel, but when done on the CPU it is done per occupied
voxel after the GPU-to-CPU bus transfer. On the GPU it is done as part of the
compact pass, before the bus transfer, and reduces the output size: instead of
transferring a u32 triangle index per voxel, the GPU transfers a u32 material ID.
The result is the same size (both u32), but the GPU version eliminates the
subsequent CPU lookup entirely.

### Material table packing is memory-conscious

The `material_table` buffer uses two u16 MaterialId values per u32 word, halving
the buffer size. The WGSL shift/mask to read a single u16 is negligible GPU cost.
For typical OBJ files (< 64 material groups), the buffer fits in a cache line.

### Single readback eliminates bus overhead

Architecture A requires reading `occupancy`, `owner_id`, and (optionally)
`color_rgba` arrays — three separate reads or a combined read of a dense structure
that includes unoccupied slots. Architecture B reads one compact AoS buffer:
`n_occupied × 16 bytes`. No unoccupied slots are transferred.

### CPU work is the minimum necessary

After Architecture B, the CPU does only what the GPU cannot: group by chunk
coordinate (which requires `div_euclid` by `CS=62`, a CPU concern) and call
into the chunk manager's palette-based write path. Both operations scale with
`n_occupied`, not with grid volume.

---

## Consequences

### Changes required

1. **`COMPACT_ATTRS_WGSL` shader** — add `material_table` binding, `g_origin`
   uniform, change per-voxel output from `(linear_index, owner_id, color_rgba)`
   to `(vx, vy, vz, material)`. Remove `out_color`.

2. **`compact_attrs.rs`** — add parameters, change return type to
   `Vec<CompactVoxel>`.

3. **`core.rs`** — add `CompactVoxel` struct.

4. **New crate `crates/greedy_voxelizer`** — CPU ingestion:
   `compact_to_chunk_writes(voxels, manager)`.

5. **`crates/wasm_greedy_mesher`** — new `init_voxelizer()` and
   `voxelize_and_apply()` methods; `WasmChunkManager.inner` wrapped in
   `Rc<RefCell<>>`.

### What does not change

- `crates/voxelizer/` core triangle intersection math — unchanged
- `crates/wasm_voxelizer/` — unchanged (reference implementation)
- `crates/greedy_mesher/` — unchanged
- `crates/wasm_obj_loader/` — unchanged (OBJ parser already produces correct
  `material_table` input)
- All existing `WasmChunkManager` method signatures — unchanged

### Why opaque_mask is not written from the GPU

WGSL has no `atomic<u64>`. The chunk manager's `opaque_mask` is `[u64; CS_P²]`
— a Y-column bitmask where multiple GPU threads writing different Y bits into the
same column word would race. Splitting each u64 into two u32 halves and issuing
paired `atomicOr` calls is fragile and encodes chunk-internal layout constants
(CS_P=64, the column stride, the +1 padding offset) into the shader. The
compaction approach already eliminates the CPU occupancy scan, which was the
dominant cost. Writing `opaque_mask` from the GPU is not worth the complexity.

---

## See Also

- `philosophy.md` — why the chunk manager's contract drives all decisions
- `design/gpu-output-contract.md` — what Architecture B requires the GPU to produce
- `design/cpu-ingestion.md` — what the CPU does with GPU output
- `impl/gpu-shader-changes.md` — exact shader modifications
- `archive/SUPERSEDED.md` — what the prior documents said and where they are preserved
