**Type:** legacy
**Status:** legacy

> **SUPERSEDED** — This document implements Architecture A (CPU-side occupancy scan and material lookup).
> Architecture B (GPU-compact) is the current design. See `docs/voxelizer-integration/`.
> Content preserved in: `impl/gpu-shader-changes.md`, `impl/greedy-voxelizer-crate.md`, `impl/wasm-bindings.md`, `impl/overview.md`.
> This file is retained as a historical record. Do not implement from it.

---

# Voxelizer → Greedy Mesh Integration: Implementation Plan

**Date:** 2026-02-21
Status: Ready to implement

---

## Related Documents

This plan is derived from and must be read alongside:

- [`voxelizer-greedy-integration-spec.md`](voxelizer-greedy-integration-spec.md) — authoritative spec; supersedes all other docs on topics it covers
- [`voxelizer-materials-state-requirements-architecture-report.md`](voxelizer-materials-state-requirements-architecture-report.md) — material pipeline requirements
- [`voxelizer-greedy-native-migration-outline.md`](voxelizer-greedy-native-migration-outline.md) — phase-by-phase migration plan
- [`voxelizer-greedy-mesher-unification-report.md`](voxelizer-greedy-mesher-unification-report.md) — problem framing and motivation
- [`adr/0005-voxelizer-to-mesher-integration.md`](adr/) — decision: attribution via owner_id lookup
- [`adr/0007-material-strategy.md`](adr/) — decision: u16 MaterialId, palette compression
- [`adr/0008-design-gap-mitigations.md`](adr/) — known gaps and mitigations (float snapping, neighbor policy)

---

## Context

The GPU voxelizer (`crates/voxelizer`) and the greedy mesh chunk manager
(`crates/greedy_mesher`) currently have no material connection. The voxelizer
produces `SparseVoxelizationOutput` with `owner_id` (per-voxel triangle index)
but hardcodes `material_ids: None`. The chunk manager has a complete
palette-based `MaterialId (u16)` system and material-aware greedy merging.

The goal is a single-call WASM API that voxelizes a mesh, attributes materials
per-voxel, and writes directly into the chunk manager — while leaving
`wasm_voxelizer` untouched as reference.

OBJ material group parsing was already added to `wasm_obj_loader` and
`apps/web/src/modules/wasmObjLoader/helpers.ts`, producing `triangleMaterials`
and `buildMaterialTable()`. This plan wires that output into the voxelizer.

---

## Key Invariants (from spec §1–§2)

These must hold at every call boundary. Violations cause silent mesh artifacts.

### VOX-SIZE
The `voxel_size` used for voxelization must exactly equal `manager.voxel_size()`.
Enforced by reading `manager.voxel_size()` inside `voxelize_and_apply` and
passing it to `VoxelGridSpec` — the caller never provides voxel_size separately.

### VOX-ALIGN (spec §1.3)
Each component of `origin_world` must be aligned to `voxel_size`:
```
(origin_world[i] / voxel_size - (origin_world[i] / voxel_size).round()).abs() < 1e-4
```
Misaligned origins place voxels between chunk boundaries, causing seam artifacts.
`voxelize_and_apply` validates this before any GPU work and returns an error on failure.

### Tie-break policy (spec §2.4)
When multiple triangles intersect a single voxel, the GPU shader writes the
**minimum triangle index** into `owner_id`. This is immutable — it is encoded
in the WGSL shader and cannot be changed at the Rust layer. Material attribution
therefore goes to whichever triangle first appears in the index buffer.

### Material reserved values (spec §2, ADR-0007)
- `MATERIAL_EMPTY = 0` — air/unoccupied. Never written to an occupied voxel.
- `MATERIAL_DEFAULT = 1` — solid with no explicit material.
- `material_table[tri]` is 1-based. `buildMaterialTable()` guarantees this.
  Guard: if `material_table[tri] == 0` (should not occur), use `MATERIAL_DEFAULT`.

### Application layer is the material authority (materials report §4.2)
The voxelizer never decides material semantics. It produces `owner_id`
(triangle provenance). The application layer supplies `material_table` and
is solely responsible for what `MaterialId` each triangle maps to.

---

## Data Flow

```
OBJ parser (already complete)
  positions, indices, triangleMaterials, materialGroupNames
          ↓
  buildMaterialTable(triangleMaterials, materialGroupNames)
          → material_table: Uint16Array   (tri_idx → MaterialId, 1-based)
          ↓
WasmChunkManager.voxelize_and_apply(
  positions, indices, material_table, origin, dims, epsilon
)
          ↓
  [VOX-ALIGN validation — before GPU work]
          ↓
  GpuVoxelizer.voxelize_surface_sparse(MeshInput, grid, store_owner=true)
          ↓
  SparseVoxelizationOutput
    owner_id[b * brick_voxels + v] = min triangle_idx per voxel (GPU tie-break)
          ↓
  sparse_to_chunk_edits(output, material_table, origin_world, manager)
    for each occupied voxel:
      tri = owner_id[b * brick_voxels + v]
      if tri == u32::MAX → MATERIAL_DEFAULT  (explicit guard before cast)
      MaterialId = material_table[tri as usize] or MATERIAL_DEFAULT
      world_pos  = origin + (brick_origin + local_xyz) * voxel_size
      edits.push((world_pos, MaterialId))
    manager.set_voxels_batch(&edits)
      ← groups by chunk, increments version once per chunk (not per voxel)
      ← dirty marking deferred until all edits processed
          ↓
  ChunkManager: dirty chunks enqueued → greedy mesher rebuilds with materials
```

**Key design decision:** `MeshInput.material_ids` stays `None`. Attribution
happens entirely in `sparse_to_chunk_edits` using `owner_id` (triangle index
from GPU) as an index into `material_table`. No GPU shader changes needed.

---

## Phase 1 — New Crate: `crates/greedy_voxelizer`

**Purpose:** Pure Rust library (no WASM, no JS). Bridges `voxelizer` output
to `greedy_mesher` chunk writes. Spec §4 prescribes this as a separate crate
to prevent coupling voxelizer core changes to greedy logic.

### Crate dependency graph

```
crates/voxelizer         (GPU compute — unchanged)
    ↓
crates/greedy_voxelizer  (NEW: conversion layer)
    ↓
crates/greedy_mesher     (chunk manager — unchanged)
    ↓
crates/wasm_greedy_mesher (WASM bindings — extended only)
```

### File layout

```
crates/greedy_voxelizer/
  Cargo.toml
  src/
    lib.rs       — pub use re-exports
    convert.rs   — sparse_to_chunk_edits
```

### `Cargo.toml`

```toml
[package]
name = "greedy_voxelizer"
version = "0.1.0"
edition = "2021"

[dependencies]
greedy_mesher = { path = "../greedy_mesher" }
voxelizer     = { path = "../voxelizer" }
```

### `src/lib.rs`

```rust
mod convert;
pub use convert::sparse_to_chunk_edits;
```

### `src/convert.rs`

```rust
use greedy_mesher::{chunk::ChunkManager, MaterialId, MATERIAL_DEFAULT};
use voxelizer::core::SparseVoxelizationOutput;

/// Convert GPU sparse voxelization output into ChunkManager voxel writes.
///
/// Preconditions (caller must enforce):
/// - `material_table.len()` == number of triangles in the original mesh
/// - `output.owner_id` is Some (voxelizer called with store_owner: true)
/// - origin_world is VOX-ALIGN validated (each component aligned to voxel_size)
/// - voxel_size matches manager.voxel_size() (VOX-SIZE invariant)
///
/// Material lookup: material_table[triangle_idx] = MaterialId (1-based u16).
/// Tie-break: GPU writes minimum triangle_idx per voxel (immutable, from shader).
/// Fallback: owner == u32::MAX, out-of-bounds, or table[tri] == 0 → MATERIAL_DEFAULT.
///
/// Returns the count of occupied voxels written.
pub fn sparse_to_chunk_edits(
    output: &SparseVoxelizationOutput,
    material_table: &[u16],
    origin_world: [f32; 3],
    manager: &mut ChunkManager,
) -> usize {
    let brick_dim       = output.brick_dim as usize;
    let brick_voxels    = brick_dim * brick_dim * brick_dim;
    let words_per_brick = (brick_voxels + 31) / 32;
    let voxel_size      = manager.voxel_size();
    let num_bricks      = output.brick_origins.len();

    let mut edits: Vec<([f32; 3], MaterialId)> = Vec::new();

    for b in 0..num_bricks {
        let brick_origin = output.brick_origins[b]; // [u32; 3], grid-space
        let occ_start    = b * words_per_brick;
        let own_start    = b * brick_voxels;

        for v in 0..brick_voxels {
            let word = output.occupancy[occ_start + v / 32];
            if (word >> (v % 32)) & 1 == 0 {
                continue;
            }

            let lx = (v / (brick_dim * brick_dim)) as u32;
            let ly = ((v / brick_dim) % brick_dim) as u32;
            let lz = (v % brick_dim) as u32;

            let world = [
                origin_world[0] + (brick_origin[0] + lx) as f32 * voxel_size,
                origin_world[1] + (brick_origin[1] + ly) as f32 * voxel_size,
                origin_world[2] + (brick_origin[2] + lz) as f32 * voxel_size,
            ];

            let material = if let Some(owner) = &output.owner_id {
                let raw = owner[own_start + v];
                // Check u32::MAX BEFORE casting — u32::MAX as usize is valid
                // but meaningless. Spec §9.4: u32::MAX means no owner.
                if raw == u32::MAX {
                    MATERIAL_DEFAULT
                } else {
                    let tri = raw as usize;
                    if tri < material_table.len() && material_table[tri] != 0 {
                        material_table[tri]
                    } else {
                        MATERIAL_DEFAULT
                    }
                }
            } else {
                MATERIAL_DEFAULT
            };

            edits.push((world, material));
        }
    }

    let count = edits.len();
    // set_voxels_batch groups edits by chunk internally and increments each
    // chunk's version exactly once — dirty marking is deferred until all
    // edits are processed (spec §10.4, §7.2).
    manager.set_voxels_batch(&edits);
    count
}
```

---

## Phase 2 — Extend `crates/wasm_greedy_mesher`

### `Cargo.toml` — new dependencies

```toml
greedy_voxelizer     = { path = "../greedy_voxelizer" }
voxelizer            = { path = "../voxelizer" }
wasm-bindgen-futures = "0.4"
glam                 = "0.27"
wgpu                 = { version = "22", features = ["webgpu"] }
```

### `WasmChunkManager` struct

The spec (§4.5) mandates wrapping `inner` in `Rc<RefCell<>>` so a mutable
borrow of the chunk manager can be obtained inside the async block after the
GPU `await` point. `&mut self` cannot be held across `await`.

```rust
// Before
pub struct WasmChunkManager {
    inner: ChunkManager,
}

// After
pub struct WasmChunkManager {
    inner: Rc<RefCell<ChunkManager>>,
    gpu_voxelizer: Option<Rc<voxelizer::gpu::GpuVoxelizer>>,
}
```

`Rc` (not `Arc`) — wgpu on WASM is single-threaded. Both fields use `Rc` so
they can be cheaply cloned into async closures. Matches the pattern in
`wasm_voxelizer/src/lib.rs` where `WasmVoxelizer` wraps `Rc<GpuVoxelizer>`.

All three constructors (`new`, `with_config`, `with_budget`) wrap the
`ChunkManager` in `Rc<RefCell<>>` and set `gpu_voxelizer: None`. All existing
WASM methods that previously accessed `self.inner.x()` now do
`self.inner.borrow_mut().x()`.

### New method: `init_voxelizer() -> Promise<bool>`

```rust
/// Initialise the embedded GPU voxelizer.
/// Must be called once before voxelize_and_apply.
/// Returns true on success, false if WebGPU is unavailable.
pub fn init_voxelizer(&mut self) -> js_sys::Promise {
    // Clone Rc before async block — self cannot be referenced after await
    let inner_ref = self.inner.clone(); // to store result after await
    future_to_promise(async move {
        match GpuVoxelizer::new(GpuVoxelizerConfig::default()).await {
            Ok(v) => {
                // store into the shared field via inner_ref or similar
                Ok(JsValue::from(true))
            }
            Err(_) => Ok(JsValue::from(false)),
        }
    })
}
```

Because `WasmChunkManager` itself is not `Clone`, the cleanest approach is to
store `gpu_voxelizer` on a separate `Rc<RefCell<Option<GpuVoxelizer>>>` field
parallel to `inner`, clone it before the async block, and assign into it after
the GPU init `await`.

### New method: `voxelize_and_apply() -> Promise<u32>`

```rust
/// Voxelise a triangle mesh and write occupied voxels into the chunk manager.
///
/// Preconditions:
/// - init_voxelizer() must have been called and returned true
/// - material_table.length must equal the number of triangles (indices.length / 3)
/// - origin must be VOX-ALIGN aligned: each component is a multiple of voxel_size
///
/// - `positions`      : flat f32 [x,y,z, x,y,z, ...]
/// - `indices`        : flat u32 triangle index array
/// - `material_table` : u16 per-triangle, material_table[tri] = MaterialId (1-based)
///                      Build with buildMaterialTable() from wasmObjLoader/helpers.ts
/// - `origin_x/y/z`   : world-space position of voxeliser grid (0,0,0)
/// - `dim_x/y/z`      : grid dimensions in voxels
/// - `epsilon`        : surface epsilon (typically 1e-4)
///
/// Returns Promise<number> — count of voxels written into the chunk manager.
pub fn voxelize_and_apply(
    &mut self,
    positions: js_sys::Float32Array,
    indices: js_sys::Uint32Array,
    material_table: js_sys::Uint16Array,
    origin_x: f32, origin_y: f32, origin_z: f32,
    dim_x: u32, dim_y: u32, dim_z: u32,
    epsilon: f32,
) -> js_sys::Promise
```

**Implementation steps inside the async block:**

1. **VOX-ALIGN check** (spec §1.3) — validate each origin component before GPU work:
   ```rust
   for i in 0..3 {
       let ratio = origin[i] / voxel_size;
       if (ratio - ratio.round()).abs() > 1e-4 {
           return Err(JsValue::from_str("origin not aligned to voxel_size"));
       }
   }
   ```
2. Copy typed arrays to owned `Vec`s — WASM memory views cannot be held across `await`.
3. Clone `Rc<GpuVoxelizer>` and `Rc<RefCell<ChunkManager>>` before async block.
4. Capture `voxel_size = manager.borrow().voxel_size()`.
5. Build `MeshInput` triangles from `positions_vec` + `indices_vec.chunks(3)`.
6. `MeshInput { triangles, material_ids: None }` — attribution is via `owner_id`.
7. Build `VoxelGridSpec { origin_world: Vec3(origin_x, origin_y, origin_z), voxel_size, dims: [dim_x, dim_y, dim_z], world_to_grid: None }`.
8. `VoxelizeOpts { epsilon, store_owner: true, store_color: false }`.
9. `output = gpu_voxelizer.voxelize_surface_sparse(&mesh, &grid, &opts).await?`.
10. `count = greedy_voxelizer::sparse_to_chunk_edits(&output, &mat_vec, [origin_x, origin_y, origin_z], &mut manager.borrow_mut())`.
11. Return `JsValue::from(count as u32)`.

---

## Key Constants (spec §9.3)

| Constant | Value | Source |
|----------|-------|--------|
| `CS` | `62` | usable voxels per chunk side |
| `CS_P` | `64` | padded chunk side (CS + 2 border) |
| `MATERIAL_EMPTY` | `0` | air/unoccupied — never written to solid voxel |
| `MATERIAL_DEFAULT` | `1` | solid with no explicit material |
| `brick_voxels` | `brick_dim³` | voxels per brick |
| `words_per_brick` | `(brick_voxels + 31) / 32` | u32 words per brick in occupancy |

---

## Data Layout Reference

| Field | Layout |
|-------|--------|
| `owner_id` | Dense: `[b * brick_voxels + v]` = triangle_idx; `u32::MAX` = no owner |
| `occupancy` | Bitpacked u32: bit `v % 32` of word `v / 32` within each brick |
| `brick_origins[b]` | `[u32; 3]` grid-space origin of brick `b` |
| `material_table[tri]` | MaterialId u16, 1-based; 0 should not occur (buildMaterialTable guarantees) |

---

## TypeScript Usage (after WASM rebuild)

```typescript
import { buildMaterialTable, parseObjFallback }
  from "../wasmObjLoader/helpers";

// 1. Initialise once after creating WasmChunkManager
const ok = await manager.init_voxelizer();
if (!ok) throw new Error("WebGPU unavailable");

// 2. Parse OBJ — now returns material group data
const { positions, indices, triangleMaterials, materialGroupNames }
  = parseObjFallback(objText);

// 3. Build material table  (tri_idx → MaterialId, 1-based)
const materialTable = buildMaterialTable(triangleMaterials, materialGroupNames);
// materialTable.length === indices.length / 3  (one entry per triangle)

// 4. Origin must be VOX-ALIGN aligned (multiple of voxel_size on each axis)
const voxelSize = manager.voxel_size();
const originX = Math.round(rawOriginX / voxelSize) * voxelSize;
const originY = Math.round(rawOriginY / voxelSize) * voxelSize;
const originZ = Math.round(rawOriginZ / voxelSize) * voxelSize;

// 5. Voxelise and write into chunk manager in one call
const count = await manager.voxelize_and_apply(
  positions,
  indices,
  materialTable,
  originX, originY, originZ,
  gridDim, gridDim, gridDim,
  1e-4
);

// 6. Normal frame update — dirty chunks rebuild with material-aware merge
manager.update(camX, camY, camZ);

// 7. Mesh contains per-vertex material_ids ready for atlas lookup
const mesh = manager.get_chunk_mesh(cx, cy, cz);
// mesh.material_ids — distinct u16 values per usemtl region
```

---

## Files to Create / Modify

| File | Action |
|------|--------|
| `crates/greedy_voxelizer/Cargo.toml` | **Create** |
| `crates/greedy_voxelizer/src/lib.rs` | **Create** |
| `crates/greedy_voxelizer/src/convert.rs` | **Create** — core attribution loop |
| `crates/wasm_greedy_mesher/Cargo.toml` | **Modify** — add 5 deps |
| `crates/wasm_greedy_mesher/src/lib.rs` | **Modify** — `Rc<RefCell<>>` wrap, field + 2 methods |

**Left untouched (reference):**

- `crates/voxelizer/` — GPU voxeliser core
- `crates/wasm_voxelizer/` — standalone WASM voxeliser
- `crates/wasm_obj_loader/` — OBJ parser (already updated)

---

## Verification

```bash
# Rust
cargo build -p greedy_voxelizer
cargo build -p wasm_greedy_mesher

# WASM binary
pnpm build:wasm
```

In the testbed with sponza.obj:

- `count > 0` after `voxelize_and_apply`
- `materialGroupNames.length > 1` (Sponza has many usemtl groups)
- After `manager.update(...)`, `get_chunk_mesh(cx, cy, cz).material_ids`
  contains multiple distinct values — confirming per-material greedy merge
- Load cube.obj (no usemtl) — all `material_ids` should be `1` (MATERIAL_DEFAULT)
