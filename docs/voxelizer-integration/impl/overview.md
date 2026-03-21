# Implementation Overview

**Type:** spec
**Status:** proposed
**Date:** 2026-02-22

---

## What This Document Covers

The file-by-file map of what needs to be built to implement Architecture B. Read
this before any of the other `impl/` documents.

---

## Crate Dependency Graph

### Before

```
crates/voxelizer         (GPU compute core)
    ↓
crates/wasm_voxelizer    (standalone WASM — unchanged reference)

crates/greedy_mesher     (chunk manager — unchanged)
    ↓
crates/wasm_greedy_mesher (WASM bindings — no voxelizer connection)
```

### After

```
crates/voxelizer         (GPU compute core — modified compact pass)
    ↓
crates/greedy_voxelizer  (NEW: CPU ingestion layer)
    ↓ also depends on
crates/greedy_mesher     (chunk manager — unchanged)
    ↓
crates/wasm_greedy_mesher (WASM bindings — extended with voxelizer methods)

crates/wasm_voxelizer    (unchanged — still depends only on crates/voxelizer)
```

`crates/wasm_voxelizer` does not depend on `crates/greedy_voxelizer`. It remains
a standalone reference implementation.

---

## File-by-File Changes

### Phase 1 — Extend GPU compact pass (`crates/voxelizer`)

| File | Action |
|------|--------|
| `crates/voxelizer/src/core.rs` | **Modify** — add `CompactVoxel` struct |
| `crates/voxelizer/src/gpu/shaders.rs` | **Modify** — update `COMPACT_ATTRS_WGSL` |
| `crates/voxelizer/src/gpu/compact_attrs.rs` | **Modify** — new parameters, new return type |

Details: `impl/gpu-shader-changes.md`

### Phase 2 — New CPU ingestion crate (`crates/greedy_voxelizer`)

| File | Action |
|------|--------|
| `crates/greedy_voxelizer/Cargo.toml` | **Create** |
| `crates/greedy_voxelizer/src/lib.rs` | **Create** |
| `crates/greedy_voxelizer/src/ingest.rs` | **Create** — `compact_to_chunk_writes` |

Details: `impl/greedy-voxelizer-crate.md`

### Phase 3 — Extend WASM bindings (`crates/wasm_greedy_mesher`)

| File | Action |
|------|--------|
| `crates/wasm_greedy_mesher/Cargo.toml` | **Modify** — add 5 new deps |
| `crates/wasm_greedy_mesher/src/lib.rs` | **Modify** — struct change, 2 new methods |

Details: `impl/wasm-bindings.md`

### Untouched

| Crate/File | Reason |
|-----------|--------|
| `crates/voxelizer/` (except the 3 files above) | Core GPU math unchanged |
| `crates/wasm_voxelizer/` | Reference implementation, must not change |
| `crates/greedy_mesher/` | Chunk manager unchanged |
| `crates/wasm_obj_loader/` | OBJ parser already complete and consistent |
| `apps/web/src/modules/wasmObjLoader/` | Already consistent with this design |
| `apps/web/src/modules/wasmVoxelizer/helpers.ts` | Re-exports; no change needed |

---

## Build Order

```bash
# Phase 1: verify GPU crate still compiles with new compact pass
cargo build -p voxelizer

# Phase 2: verify new ingestion crate compiles
cargo build -p greedy_voxelizer

# Phase 3: verify WASM crate compiles with new methods
cargo build -p wasm_greedy_mesher

# Full WASM binary
pnpm build:wasm
```

Build Phase 1 before Phase 2 (greedy_voxelizer depends on voxelizer's new types).
Build Phase 2 before Phase 3 (wasm_greedy_mesher depends on greedy_voxelizer).

---

## Verification

### Rust-level

After Phase 2, `greedy_voxelizer` has no WASM dependency and can be tested with
a wgpu Vulkan backend in native tests:

```bash
cargo test -p greedy_voxelizer
```

### TypeScript integration test (sponza.obj)

```typescript
// init
await manager.init_voxelizer();

// parse
const { positions, indices, triangleMaterials, materialGroupNames }
    = parseObjFallback(sponzaObjText);
const materialTable = buildMaterialTable(triangleMaterials, materialGroupNames);

// voxelize
const count = await manager.voxelize_and_apply(
    positions, indices, materialTable,
    ox, oy, oz,
    gridDim, gridDim, gridDim,
    1e-4
);

// assertions
assert(count > 0, 'no voxels written');
assert(materialGroupNames.length > 1, 'sponza should have multiple usemtl groups');

manager.update(camX, camY, camZ);
const mesh = manager.get_chunk_mesh(cx, cy, cz);
const distinctMaterials = new Set(mesh.material_ids);
assert(distinctMaterials.size > 1, 'mesh should have multiple materials');

// regression: cube.obj with no usemtl → all MATERIAL_DEFAULT
const { positions: cubePos, indices: cubeIdx, triangleMaterials: cubeMat,
        materialGroupNames: cubeGroups }
    = parseObjFallback(cubeObjText);
const cubeTable = buildMaterialTable(cubeMat, cubeGroups);
await manager.voxelize_and_apply(cubePos, cubeIdx, cubeTable, ...);
const cubeMesh = manager.get_chunk_mesh(...);
const cubeDistinct = new Set(cubeMesh.material_ids);
assert(cubeDistinct.size === 1 && cubeDistinct.has(1),
    'cube with no usemtl should produce only MATERIAL_DEFAULT');
```
