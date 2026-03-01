# Coordinate Frame Specification

Date: February 22, 2026
Status: Authoritative

Source: Refined from `archive/voxelizer-greedy-integration-spec.md` §1
(the coordinate derivation from that document is preserved here verbatim;
the implementation notes are updated for Architecture B).

---

## The Three Coordinate Spaces

Three distinct coordinate spaces are in play. They must be formally separated to
avoid confusion in any implementation.

### Space A — World Space (float)

The common 3-D Euclidean space shared by the scene, the camera, and all objects.
Units are arbitrary engine units (e.g. metres). Positions are `Vec3 (f32)`.

### Space B — Voxelizer Grid Space (non-negative integer)

The coordinate space internal to a single voxelization request. Defined by a
`VoxelGridSpec`:

```rust
struct VoxelGridSpec {
    origin_world: Vec3,         // World-space origin of grid voxel (0,0,0)
    voxel_size:   f32,          // Side length of each grid voxel in world units
    dims:         [u32; 3],     // Grid extents: gx in [0, dims[0]), etc.
    world_to_grid: Option<Mat4>,
}
```

Source: `crates/voxelizer/src/core.rs:4–9`.

The forward mapping World → Grid Space:

```
G = (W - origin_world) / voxel_size
```

Source: `crates/voxelizer/src/core.rs:34–40`.

Grid coordinates are always non-negative (`u32`). The voxelizer clamps triangle
projections to `[0, dims[i])`.

### Space C — Greedy Global Voxel Space (signed integer)

The coordinate space of the greedy chunk manager. Voxel addresses are signed
`[i32; 3]`, supporting negative coordinates for worlds that extend in all
directions. The chunk at chunk-coordinate `(cx, cy, cz)` contains:

```
vx in [cx * CS, (cx+1) * CS)
vy in [cy * CS, (cy+1) * CS)
vz in [cz * CS, (cz+1) * CS)
```

where `CS = 62` (usable voxels per chunk side).

Source: `crates/greedy_mesher/src/core.rs:16`.

---

## Formal Mapping: Grid Space → Greedy Global Voxel Space

**Claim:** Assuming the same `voxel_size` is used in both systems, the signed
global voxel index `V` corresponding to voxelizer grid voxel `G = (gx, gy, gz)` is:

```
V = G + G_origin

where G_origin = floor(origin_world / voxel_size)  (component-wise)
```

**Derivation:**

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

   The `floor` is needed only because floating-point `origin_world / voxel_size`
   may have a fractional part. When VOX-ALIGN holds, `floor` is a no-op.

**Implementation in Architecture B:** `G_origin` is computed once in Rust inside
`voxelize_and_apply` and passed to the GPU as the `g_origin` uniform. The GPU
applies `V = G + G_origin` per occupied voxel in the compact pass shader. No CPU
code performs this calculation at per-voxel granularity.

```rust
// Computed once per voxelization call in Rust:
let g_origin = [
    (origin_world[0] / voxel_size).floor() as i32,
    (origin_world[1] / voxel_size).floor() as i32,
    (origin_world[2] / voxel_size).floor() as i32,
];
```

---

## Invariant VOX-ALIGN

**Definition:** For each component `i ∈ {x, y, z}`:

```
origin_world[i] mod voxel_size == 0
```

i.e. `origin_world` is an exact multiple of `voxel_size` in every component.

**Consequence of violation:** If `origin_world[i] / voxel_size` has a fractional
part `ε ∈ (0, 1)`, the world-space cube of grid voxel `(gx, gy, gz)` straddles
two adjacent greedy voxels. The voxelizer assigns occupancy to its grid voxel,
but the greedy system's boundary is offset by `ε * voxel_size`. Surfaces near
a chunk boundary will be written into the wrong greedy voxel, producing seam
artifacts and incorrect mesh topology.

**Enforcement:** `voxelize_and_apply` validates VOX-ALIGN before any GPU work
begins. If violated, the function rejects with an error.

**Caller responsibility (TypeScript):** Snap the origin to the nearest voxel
grid point before calling:

```typescript
const snap = (v: number, size: number) => Math.round(v / size) * size;
const originX = snap(rawOriginX, manager.voxel_size());
const originY = snap(rawOriginY, manager.voxel_size());
const originZ = snap(rawOriginZ, manager.voxel_size());
```

**Validation code (Rust):**

```rust
fn check_alignment(origin_world: [f32; 3], voxel_size: f32) -> Result<(), String> {
    for i in 0..3 {
        let ratio = origin_world[i] / voxel_size;
        if (ratio - ratio.round()).abs() > 1e-4 {
            return Err(format!(
                "origin_world[{}]={} is not aligned to voxel_size={}; \
                 chunk boundary alignment not guaranteed",
                i, origin_world[i], voxel_size
            ));
        }
    }
    Ok(())
}
```

Source: `archive/voxelizer-greedy-integration-spec.md` §1.3.

---

## Invariant VOX-SIZE

**Definition:** The `voxel_size` used for voxelization must exactly equal
`manager.voxel_size()`.

**Enforcement:** `voxelize_and_apply` reads `voxel_size` from the chunk manager
at call entry — the caller does not provide `voxel_size` as a parameter. This
makes it structurally impossible to pass the wrong value.

Source: `crates/wasm_greedy_mesher/src/lib.rs` (`RebuildConfig.voxel_size`).

---

## Chunk Coordinate Conversions

Converting from global signed voxel `(vx, vy, vz)` to chunk coordinate and
local offset uses Euclidean division:

```rust
let cs = 62i32;
let cx = vx.div_euclid(cs);    // chunk coordinate
let lx = vx.rem_euclid(cs);    // local [0, 62) — safe for negative vx
```

Source: `crates/greedy_mesher/src/chunk/coord.rs:82–116`.

Euclidean division is required (not integer division) because negative global
coordinates are valid. For `vx = -1`:
- `div_euclid(-1, 62) = -1` (chunk at x=-1)
- `rem_euclid(-1, 62) = 61` (local slot 61 in that chunk)
- Integer division would give `(-1)/62 = 0` and `(-1)%62 = -1` — both wrong.
