# Material Pipeline Specification

**Type:** spec
**Status:** proposed
**Date:** 2026-02-22

Sources: `archive/voxelizer-greedy-integration-spec.md` §2;
`archive/voxelizer-chunk-native-output-design-requirements.md` §material_table;
`archive/voxelizer-materials-state-requirements-architecture-report.md` §§2.4, 3

---

## Overview: End-to-End Material Flow

```
OBJ file  →  parse_obj / parseObjFallback
               triangleMaterials: Uint32Array  (one entry per triangle,
               materialGroupNames: string[]     value = group index 0-based)
                         ↓
            buildMaterialTable(triangleMaterials, materialGroupNames)
               material_table: Uint16Array     (one entry per triangle,
                                                value = MaterialId 1-based)
                         ↓
            CPU packs material_table before GPU upload:
               packed: Vec<u32>               (two u16 per u32 word)
                         ↓
            GPU compact pass reads packed table,
            resolves owner_id → MaterialId per occupied voxel
                         ↓
            CompactVoxel[].material            (u32, carries MaterialId)
                         ↓
            CPU ingestion → set_voxel_raw(lx, ly, lz, material)
                         ↓
            ChunkManager palette storage      (PaletteMaterials, u16)
```

---

## MaterialId Type

```rust
pub type MaterialId = u16;           // crates/greedy_mesher/src/core.rs:5
pub const MATERIAL_EMPTY:   u16 = 0; // air/unoccupied
pub const MATERIAL_DEFAULT: u16 = 1; // solid with no explicit material
```

Source: `crates/greedy_mesher/src/core.rs:5, 8–10`.

`MaterialId` is 1-based for solid voxels. `0` is reserved for empty/air. An
implementer must never write `0` to an occupied voxel.

---

## `material_table` Definition

```
material_table: &[u16]

  length = number of triangles in the mesh (indices.len() / 3)
  material_table[tri_index] = MaterialId to assign to voxels whose winner is tri_index
```

**Contract:**
- Length must equal the triangle count. A mismatch is a caller error.
- Values are 1-based MaterialIds. `0` must not appear (guaranteed by
  `buildMaterialTable`).
- A triangle with no usemtl group maps to MaterialId 1 (MATERIAL_DEFAULT).

---

## `buildMaterialTable` — Application Layer Contract

```typescript
// apps/web/src/modules/wasmObjLoader/helpers.ts
export const buildMaterialTable = (
    triangleMaterials: Uint32Array,   // one entry per triangle, 0-based group index
    _materialGroupNames: string[]     // group names (unused in current impl)
): Uint16Array => {
    const table = new Uint16Array(triangleMaterials.length);
    for (let i = 0; i < triangleMaterials.length; i++) {
        table[i] = triangleMaterials[i] + 1;   // group 0 → MaterialId 1
    }
    return table;
};
```

**Guarantee:** No zeros in the output. Group index 0 (default/unassigned) maps
to MaterialId 1 (MATERIAL_DEFAULT). Group index N maps to MaterialId N+1.

**Implication:** MaterialId assignment is sequential by the order material groups
appear in the OBJ file. Groups that appear earlier have lower MaterialIds.
Stable OBJ files produce stable MaterialIds across calls, which minimizes palette
repacks in the chunk manager.

---

## `material_table` Packing for GPU

The CPU packs the `Uint16Array` into a `Vec<u32>` before uploading to the GPU.
Two u16 values per u32 word:

```rust
let packed: Vec<u32> = table.chunks(2).map(|pair| {
    (pair[0] as u32) | ((pair.get(1).copied().unwrap_or(0) as u32) << 16)
}).collect();
```

For triangle index `tri`, the GPU reads:

```wgsl
let word  = tri >> 1u;              // which u32 word
let shift = (tri & 1u) << 4u;      // 0 or 16
let mat   = (material_table[word] >> shift) & 0xFFFFu;
```

This halves the buffer size. The shift/mask is a negligible GPU cost. For typical
OBJ files (< 64 material groups, triangle count ~10K–100K), the entire packed
table fits in GPU L2 cache.

---

## Tie-Break Policy

When multiple triangles intersect the same voxel, the GPU shader keeps the one
with the **minimum triangle index**:

```wgsl
if (tri < best) { best = tri; }
```

Source: `crates/voxelizer/src/gpu/shaders.rs:205`.

This is the existing GPU behavior and **must not be changed.** The conversion from
`owner_id → MaterialId` inherits this policy automatically.

**Implication for callers:** If two material groups overlap in the mesh (e.g. a
detail mesh layered over a base mesh), the triangle that appears first in the index
buffer wins. Callers must order triangles such that lower-indexed triangles have
higher semantic priority in overlap regions. This is an application-layer concern;
the voxelizer and ingestion code do not require changes to support it.

---

## Sentinel Values

| Value | Meaning | CPU handling |
|-------|---------|--------------|
| `u32::MAX` in `owner_id` (GPU) | No triangle claimed this voxel | GPU writes `material = 0xFFFFFFFF` in compact output |
| `0xFFFFFFFF` in `CompactVoxel.material` | Unresolved owner | CPU uses `MATERIAL_DEFAULT (1)` |
| `0` in `CompactVoxel.material` | Should not occur; fallback guard | CPU uses `MATERIAL_DEFAULT (1)` |

**CPU sentinel check:**

```rust
let mat = if v.material == 0xFFFF_FFFF || v.material == 0 {
    MATERIAL_DEFAULT
} else {
    v.material as MaterialId
};
```

The `u32::MAX` sentinel must be checked **before casting** to avoid a valid
(but meaningless) `usize` index.

---

## Who Is the Material Authority

The application layer (TypeScript, above the WASM boundary) is the sole authority
for material assignment. It knows which mesh triangles belong to which material
group. It constructs `material_table` at mesh-load time and passes it to
`voxelize_and_apply`.

The voxelizer (`crates/voxelizer`) has no knowledge of material semantics. It
records triangle provenance (`owner_id`) during voxelization. Material meaning
is assigned after the fact by the table the caller provides.

The chunk manager (`crates/greedy_mesher`) receives `MaterialId` values and
stores them. It does not validate or interpret them beyond distinguishing
`MATERIAL_EMPTY (0)` from non-empty values.

---

## `u16` Range Adequacy

`MaterialId = u16` supports 65,536 distinct values, with 0 reserved. This gives
65,535 usable material slots. The current use case — per-usemtl material groups
on a single OBJ mesh — will not approach this limit (typical OBJ files have 5–50
groups; Sponza has ~25). If a future use case requires more, the palette layer
(`crates/greedy_mesher/src/chunk/palette_materials.rs:44–56`) handles compression
internally; only the `MaterialId` typedef would need to widen, which is a
single-line change.
