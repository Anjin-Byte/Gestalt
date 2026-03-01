# GPU Shader Changes

Date: February 22, 2026
Status: Authoritative

Files modified:
- `crates/voxelizer/src/core.rs`
- `crates/voxelizer/src/gpu/shaders.rs` (`COMPACT_ATTRS_WGSL`)
- `crates/voxelizer/src/gpu/compact_attrs.rs`

---

## 1. Add `CompactVoxel` to `core.rs`

The `CompactVoxel` struct is the return type of the updated compact pass and the
input type for the CPU ingestion layer. Add it to `crates/voxelizer/src/core.rs`:

```rust
/// One occupied voxel from the GPU compact pass.
/// AoS layout: 16 bytes, naturally aligned.
/// `material == 0xFFFFFFFF` means unresolved owner — CPU substitutes MATERIAL_DEFAULT.
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct CompactVoxel {
    pub vx:       i32,   // global voxel X (g_origin.x + gx)
    pub vy:       i32,   // global voxel Y (g_origin.y + gy)
    pub vz:       i32,   // global voxel Z (g_origin.z + gz)
    pub material: u32,   // MaterialId as u32 (1-based); 0xFFFFFFFF = unresolved
}
```

---

## 2. Update `COMPACT_ATTRS_WGSL` in `shaders.rs`

The compact shader currently outputs `(linear_index, owner_id, color_rgba)`.
It must be changed to output `(vx, vy, vz, material)`.

### 2a. Add `material_table` binding

Add after the existing storage buffer bindings:

```wgsl
@group(0) @binding(N) var<storage, read> material_table: array<u32>;
// Two u16 MaterialId values packed per u32 word.
// Binding number N = next available slot after owner_id and color_rgba.
```

### 2b. Add `g_origin` to `CompactAttrsParams`

The `CompactAttrsParams` uniform struct gains a new field:

```wgsl
// Before:
struct CompactAttrsParams {
    grid_dims:    vec3<u32>,
    _pad0:        u32,
    // ... existing fields
}

// After — add g_origin:
struct CompactAttrsParams {
    grid_dims:    vec3<u32>,
    _pad0:        u32,
    // ... existing fields ...
    g_origin:     vec3<i32>,
    _pad_origin:  u32,
}
```

The Rust-side `CompactAttrsParams` struct must be updated to match.

### 2c. Replace the per-voxel write block

Locate the block that writes `out_indices`, `out_owner`, `out_color`:

```wgsl
// BEFORE — remove this:
out_indices[idx] = linear_index;
out_owner[idx]   = owner_id[attr_index];
out_color[idx]   = color_rgba[attr_index];
```

Replace with:

```wgsl
// AFTER:
let raw_owner   = owner_id[attr_index];
let material_id = select(
    0xFFFFFFFFu,
    (material_table[raw_owner >> 1u] >> ((raw_owner & 1u) << 4u)) & 0xFFFFu,
    raw_owner != 0xFFFFFFFFu && raw_owner < arrayLength(&material_table) * 2u
);
out_vx[idx]       = i32(origin.x + vx) + params.g_origin.x;
out_vy[idx]       = i32(origin.y + vy) + params.g_origin.y;
out_vz[idx]       = i32(origin.z + vz) + params.g_origin.z;
out_material[idx] = material_id;
```

**Material lookup explained:**
- `raw_owner >> 1u` — which u32 word in the packed table
- `(raw_owner & 1u) << 4u` — shift: 0 for even triangle index, 16 for odd
- `& 0xFFFFu` — extract the lower or upper u16

**Sentinel guard:**
- `raw_owner != 0xFFFFFFFFu` — `u32::MAX` means no triangle claimed this voxel
- `raw_owner < arrayLength(&material_table) * 2u` — bounds check (packed table
  has `len/2` u32 words, each holding 2 u16 entries)
- `select(0xFFFFFFFFu, computed, cond)` — if either guard fails, output sentinel

**Coordinate computation:**
- `origin.x + vx` — voxelizer grid-space X (non-negative u32)
- `+ params.g_origin.x` — adds the signed global offset to produce global i32 X

### 2d. Remove `out_color`

The `color_rgba` buffer binding and `out_color[idx]` write are removed entirely.
Debug coloring has no role in chunk manager ingestion and increases bus traffic.
The `store_color` option in `VoxelizeOpts` can remain for the legacy path but
is ignored in the new compact pass.

### 2e. Update output buffer declarations

Replace:

```wgsl
@group(0) @binding(A) var<storage, read_write> out_indices: array<u32>;
@group(0) @binding(B) var<storage, read_write> out_owner: array<u32>;
@group(0) @binding(C) var<storage, read_write> out_color: array<u32>;
```

With:

```wgsl
@group(0) @binding(A) var<storage, read_write> out_vx: array<i32>;
@group(0) @binding(B) var<storage, read_write> out_vy: array<i32>;
@group(0) @binding(C) var<storage, read_write> out_vz: array<i32>;
@group(0) @binding(D) var<storage, read_write> out_material: array<u32>;
```

Or, if the implementation uses an AoS buffer (preferred — see
`design/gpu-output-contract.md`):

```wgsl
struct CompactVoxelGpu {
    vx:       i32,
    vy:       i32,
    vz:       i32,
    material: u32,
}

@group(0) @binding(A) var<storage, read_write> out_voxels: array<CompactVoxelGpu>;
```

The AoS layout is preferred: one binding, one readback. Matches `CompactVoxel`
in `core.rs` byte-for-byte (bytemuck `Pod` cast works directly).

---

## 3. Update `compact_attrs.rs`

### 3a. New function signature

```rust
pub async fn compact_sparse_attributes(
    // existing params:
    device:         &wgpu::Device,
    queue:          &wgpu::Queue,
    occupancy_buf:  &wgpu::Buffer,
    owner_id_buf:   &wgpu::Buffer,
    brick_origins:  &[[u32; 3]],
    brick_dim:      u32,
    grid_dims:      [u32; 3],
    n_bricks:       u32,
    // new params:
    material_table: &[u32],         // packed: two u16 per u32
    g_origin:       [i32; 3],       // global voxel offset
) -> Vec<CompactVoxel>              // changed return type
```

### 3b. New buffer: `material_table_buf`

```rust
let material_table_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
    label: Some("material_table"),
    contents: bytemuck::cast_slice(material_table),
    usage: wgpu::BufferUsages::STORAGE,
});
```

### 3c. Updated `CompactAttrsParams` Rust struct

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CompactAttrsParams {
    // ... existing fields ...
    g_origin:    [i32; 3],
    _pad_origin: u32,
}
```

### 3d. Readback and return type

The readback buffer is now sized for `max_occupied * size_of::<CompactVoxel>()`.
After the copy pass resolves:

```rust
let data = readback_buf.slice(..).get_mapped_range();
let voxels: Vec<CompactVoxel> = bytemuck::cast_slice::<u8, CompactVoxel>(&data)
    .iter()
    .copied()
    .collect();
drop(data);
readback_buf.unmap();
voxels
```

The returned `Vec<CompactVoxel>` has `n_occupied` entries (from the atomic counter).

---

## 4. CPU-side: Packing `material_table` Before Upload

In `voxelize_and_apply` (Rust, `crates/wasm_greedy_mesher/src/lib.rs`), pack the
`Uint16Array` from JS before calling `compact_sparse_attributes`:

```rust
let mat_vec: Vec<u16> = material_table_js.to_vec();
let mat_packed: Vec<u32> = mat_vec.chunks(2).map(|pair| {
    (pair[0] as u32) | ((pair.get(1).copied().unwrap_or(0) as u32) << 16)
}).collect();
```

---

## 5. Computing `g_origin`

Also in `voxelize_and_apply`, before GPU dispatch:

```rust
let voxel_size = self.inner.borrow().voxel_size();
let g_origin = [
    (origin_x / voxel_size).floor() as i32,
    (origin_y / voxel_size).floor() as i32,
    (origin_z / voxel_size).floor() as i32,
];
```

`g_origin` is passed to `compact_sparse_attributes` alongside the packed material
table. It is an implementation detail of the Rust layer; the TypeScript caller
only provides `origin_world` as before.
