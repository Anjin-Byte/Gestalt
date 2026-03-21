# GPU Output Contract

**Type:** spec
**Status:** proposed
**Date:** 2026-02-22

Source: `archive/voxelizer-chunk-native-output-design-requirements.md` §§GPU Output Contract, Required Changes

---

## What This Document Defines

The contract between the GPU compact pass and the CPU ingestion layer. CPU code
may rely on these guarantees without additional defensive checks (beyond sentinel
handling listed below).

---

## Output Format: `CompactVoxel`

The compact pass produces a single AoS buffer of length `n_occupied`. Each entry
is one 16-byte struct:

```wgsl
struct CompactVoxel {
    vx:       i32,   // global voxel X  (g_origin.x + gx)
    vy:       i32,   // global voxel Y  (g_origin.y + gy)
    vz:       i32,   // global voxel Z  (g_origin.z + gz)
    material: u32,   // MaterialId as u32 (1-based u16 value, 0 = MATERIAL_DEFAULT fallback)
                     // 0xFFFFFFFF if owner was unresolved (sentinel — see below)
}
```

**Rust equivalent:**

```rust
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct CompactVoxel {
    pub vx:       i32,
    pub vy:       i32,
    pub vz:       i32,
    pub material: u32,
}
```

---

## Guarantees

**G1 — Only occupied voxels appear.**
Every entry in the compact output corresponds to a voxel that was occupied
(its occupancy bit was set). Empty voxel slots are not present. `n_occupied`
is the exact count of entries.

**G2 — Coordinates are global signed integers.**
`(vx, vy, vz)` are signed `i32` values in the greedy chunk manager's global voxel
space. The GPU computes them as `g_origin + grid_xyz` before output. No further
coordinate transformation is required on the CPU.

**G3 — Material is resolved.**
The GPU has already performed `material_table[owner_id]` lookup. The CPU receives
the final MaterialId, not a triangle index. Exception: sentinel case below.

**G4 — Ordering is non-deterministic.**
The GPU atomic counter produces entries in thread-execution order, which varies
across dispatches. CPU code must not assume any ordering. Grouping by chunk
coordinate is the CPU's first operation.

**G5 — Single readback.**
All occupied voxels in one dispatch cross the bus in one copy. No second readback
for materials, coordinates, or debug data separately.

---

## Sentinel: Unresolved Owner

When `owner_id` for an occupied voxel is `u32::MAX` (the GPU's initialization
value — meaning no triangle was recorded as the owner despite the voxel being
marked occupied), the GPU writes:

```
material = 0xFFFFFFFF
```

The CPU must check for this sentinel and substitute `MATERIAL_DEFAULT (1)`:

```rust
let mat = if v.material == 0xFFFF_FFFF || v.material == 0 {
    MATERIAL_DEFAULT
} else {
    v.material as MaterialId
};
```

The `material == 0` check guards against any path that might produce
`MATERIAL_EMPTY`; it should not occur in practice but the cost of the check is zero.

---

## GPU Inputs Required by This Contract

### `material_table` buffer

```wgsl
@group(0) @binding(N) var<storage, read> material_table: array<u32>;
```

**Packing:** two `u16` MaterialId values per `u32` word. Triangle index `tri` maps to:

```wgsl
let word  = tri >> 1u;
let shift = (tri & 1u) << 4u;          // 0 or 16
let mat   = (material_table[word] >> shift) & 0xFFFFu;
```

**CPU-side packing before upload:**

```rust
let packed: Vec<u32> = table.chunks(2).map(|pair| {
    (pair[0] as u32) | ((pair.get(1).copied().unwrap_or(0) as u32) << 16)
}).collect();
```

**Bounds guard in shader:**

```wgsl
let material_id = select(
    0xFFFFFFFFu,
    (material_table[raw_owner >> 1u] >> ((raw_owner & 1u) << 4u)) & 0xFFFFu,
    raw_owner != 0xFFFFFFFFu && raw_owner < arrayLength(&material_table) * 2u
);
```

The out-of-bounds guard (`raw_owner < arrayLength * 2u`) prevents a bad triangle
index from reading past the end of the table.

### `g_origin` uniform

Added to `CompactAttrsParams`:

```wgsl
g_origin: vec3<i32>,
_pad:     u32,
```

**Computation (Rust, inside `voxelize_and_apply` before GPU dispatch):**

```rust
let g_origin = [
    (origin_world[0] / voxel_size).floor() as i32,
    (origin_world[1] / voxel_size).floor() as i32,
    (origin_world[2] / voxel_size).floor() as i32,
];
```

`g_origin` is the signed global voxel coordinate of the voxelizer grid's
world-space origin. It converts grid-space `(gx, gy, gz)` (non-negative u32) to
global voxel space (signed i32) via `V = G + G_origin`. See
`spec/coordinate-frames.md` for the full derivation.

**The TypeScript caller passes `origin_world` as before. Computing `g_origin`
from it is a Rust implementation detail and is not part of the JS API.**

---

## Per-Voxel GPU Write (Updated Shader Logic)

Current (before this change):

```wgsl
out_indices[idx] = linear_index;
out_owner[idx]   = owner_id[attr_index];
out_color[idx]   = color_rgba[attr_index];
```

After this change:

```wgsl
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

`out_color` is removed entirely — it was debug-only and has no role in chunk
manager ingestion.

---

## Why AoS Over SoA

An SoA layout (`out_vx[]`, `out_vy[]`, `out_vz[]`, `out_material[]`) would
require four separate readback copies or four separate GPU buffers. AoS
`CompactVoxel[]` uses one buffer, one readback copy, and groups each voxel's
data spatially — better cache behavior during CPU grouping (the CPU reads
`vx, vy, vz, material` for one voxel before moving to the next).

16 bytes per voxel is natural alignment with no padding waste.
