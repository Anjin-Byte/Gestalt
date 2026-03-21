# Material System

Global material table design, `MaterialEntry` layout, per-chunk palette protocol, and
how material properties flow through ingest, traversal, and raster stages.

Related: [[chunk-field-registry]] (authoritative palette fields), [[pipeline-stages]]
(R-5, R-6, I-3 consumers), [[edit-protocol]] (material table invalidation).

---

## Design Basis: Five Runtime Queries

The material system is shaped by the queries the runtime performs on every frame, not by
what producers find convenient to emit. Working from hottest to coldest:

| Query | Stage | What it needs |
|---|---|---|
| What radiance does this hit voxel emit? | R-6 cascade ray hit | `emissive_rgb` |
| What is the surface appearance at this pixel? | R-5 fragment | `albedo_rgb`, `roughness` |
| Does this chunk contain any emissive voxels? | I-3 summary rebuild | Boolean scan of palette |
| Can adjacent faces be merged? | R-1 greedy mesher | Per-voxel material identity (palette index) |
| How much light passes through? *(future)* | R-6 segment stream | `opacity` |

Everything else — production conventions, importer formats, artist workflow — adapts to
serve these queries. Not the other way around.

---

## Architecture Decision: Global Material Table

`MaterialId` (u16) is an index into a **scene-global** `material_table[]`. Per-chunk
`chunk_palette_buf` stores global MaterialId values, not embedded property structs.

```
material_table: array<MaterialEntry, MAX_MATERIALS>   // scene-global, CPU-written
chunk_palette_buf[slot]: packed u16 array             // MaterialId values per palette entry
chunk_index_buf[slot]:   bitpacked u32 array          // per-voxel palette index
palette_meta[slot]:      u32                          // palette_size: u16, bits_per_entry: u8, _pad: u8
```

### Why Not Per-Chunk Inline Properties

Per-chunk inline (embedding properties directly in `chunk_palette_buf` entries) looks
appealing because it removes the global table indirection at the cost of property changes
requiring a scan.

The tradeoff is wrong:

- **Hot path** (R-5 per-fragment, R-6 per-hit): one additional buffer read per query.
  Both paths are already bounded by rasterization and DDA traversal respectively. One
  16-byte cache-line read is negligible against that cost.

- **Cold path** (material property change): under a global table, one 16-byte write to
  `material_table[material_id]` updates the property everywhere. Under per-chunk inline,
  every resident chunk referencing that material needs its `chunk_palette_buf` rewritten —
  up to N_SLOTS buffer writes, each requiring finding the material's palette offset in that
  chunk. At 1024 slots, this is a non-trivial update cost for a common material change.

Global table + brute-force `has_emissive` sweep on material change is the correct default
at this scale. This is what GigaVoxels, GVDB, NanoVDB, and Minecraft all use.

---

## MaterialEntry Layout

16 bytes per entry, 4-byte aligned. All fields use f16 half-precision packed into u32 words.

```
// WGSL storage layout
struct MaterialEntry {
    albedo_rg:          u32,  // bits 0–15: albedo R (f16), bits 16–31: albedo G (f16)
    albedo_b_roughness: u32,  // bits 0–15: albedo B (f16), bits 16–31: roughness (f16)
    emissive_rg:        u32,  // bits 0–15: emissive R (f16), bits 16–31: emissive G (f16)
    emissive_b_opacity: u32,  // bits 0–15: emissive B (f16), bits 16–31: opacity (f16)
}
```

```rust
// Rust CPU layout — must match GPU struct exactly
#[repr(C, align(4))]
pub struct MaterialEntry {
    pub albedo_rg:          u32,  // half2: (albedo.r, albedo.g)
    pub albedo_b_roughness: u32,  // half2: (albedo.b, roughness)
    pub emissive_rg:        u32,  // half2: (emissive.r, emissive.g)
    pub emissive_b_opacity: u32,  // half2: (emissive.b, opacity)
}
```

### Field Semantics

| Field | Range | Default | Notes |
|---|---|---|---|
| `albedo_rgb` | [0, 1] per channel | (0.5, 0.5, 0.5) | Linear sRGB diffuse base color |
| `roughness` | [0, 1] | 0.5 | PBR roughness; 0 = mirror, 1 = fully rough |
| `emissive_rgb` | [0, ∞) | (0, 0, 0) | HDR radiance; f16 range sufficient for all practical emissive strengths |
| `opacity` | [0, 1] | 1.0 | 1.0 = fully opaque; 0.0 = fully transparent (future transmittance) |

Emissive is f16 × 3, matching the RGBA16F cascade atlas format. Accumulation in R-6
(`radiance += transparency * emissive_rgb`) requires no conversion.

### WGSL Unpack Helpers

```wgsl
fn mat_albedo(e: MaterialEntry) -> vec3<f32> {
    let rg = unpack2x16float(e.albedo_rg);
    let bk = unpack2x16float(e.albedo_b_roughness);
    return vec3<f32>(rg.x, rg.y, bk.x);
}

fn mat_roughness(e: MaterialEntry) -> f32 {
    return unpack2x16float(e.albedo_b_roughness).y;
}

fn mat_emissive(e: MaterialEntry) -> vec3<f32> {
    let rg = unpack2x16float(e.emissive_rg);
    let bo = unpack2x16float(e.emissive_b_opacity);
    return vec3<f32>(rg.x, rg.y, bo.x);
}

fn mat_opacity(e: MaterialEntry) -> f32 {
    return unpack2x16float(e.emissive_b_opacity).y;
}

// Fast emissive test used by I-3 summary rebuild.
// True if any emissive channel is non-zero. No float unpacking needed.
fn mat_is_emissive(e: MaterialEntry) -> bool {
    return (e.emissive_rg != 0u) || ((e.emissive_b_opacity & 0xFFFFu) != 0u);
}
```

### Reserved MaterialId Values

| Value | Meaning |
|---|---|
| `0` | `MATERIAL_EMPTY` — void/air; must never be rendered or emitted as a hit |
| `1` | `MATERIAL_DEFAULT` — fallback for unassigned voxels |
| `2+` | Producer-assigned material IDs |

---

## Per-Chunk Palette Protocol

### What `chunk_palette_buf` Contains

Each palette entry is a MaterialId (u16). Entries are packed two per u32:

```
palette_entry[i] = (chunk_palette_buf[slot][i >> 1] >> ((i & 1) * 16)) & 0xFFFFu
```

This is a compression structure, not a property cache. The MaterialId is the identity;
the global `material_table` resolves it to properties.

### `palette_meta` — Per-Slot Palette Descriptor

A new per-slot field alongside the existing pool entries:

```
palette_meta[slot]: u32
  bits  0–15: palette_size      (u16) number of entries in this chunk's palette
  bits 16–23: bits_per_entry    (u8)  index bit width: 1, 2, 4, or 8
  bits 24–31: reserved
```

`bits_per_entry` is stored explicitly rather than derived from `palette_size` at runtime.
GPU shaders must not perform `ceil(log2(palette_size))` in the hot path. CPU writes
`bits_per_entry` during chunk upload; it is authoritative.

`palette_meta` is an authoritative field: written by CPU at chunk load, updated on palette
resize, never touched by rebuild passes.

### How R-6 Resolves Material at a Hit Voxel

```wgsl
// After DDA confirms hit at (slot, lx, ly, lz)
let voxel_index: u32 = lx * 64u * 64u + ly * 64u + lz;
let bpe = (palette_meta[slot] >> 16u) & 0xFFu;         // bits_per_entry
let mask = (1u << bpe) - 1u;
let word = chunk_index_buf[slot][voxel_index * bpe / 32u];
let bit_off = (voxel_index * bpe) % 32u;
let palette_idx = (word >> bit_off) & mask;

let mat_id = (chunk_palette_buf[slot][palette_idx >> 1u] >> ((palette_idx & 1u) * 16u)) & 0xFFFFu;
let entry = material_table[mat_id];
let emissive = mat_emissive(entry);
radiance += transparency * emissive;
```

Material fetch only happens after `opaque_mask` confirms the hit. It is never part of the
DDA inner loop. See [[traversal-acceleration]] — Invariant 1.

### How R-5 Reads Surface Appearance

The greedy mesher resolves `palette_index → MaterialId` during mesh generation and emits
`material_id: u16` directly into the vertex buffer. The R-5 fragment shader does not
perform palette lookup:

```wgsl
// vertex_material_id is the global MaterialId emitted per quad by the greedy mesher
let entry = material_table[vertex_material_id];
let albedo = mat_albedo(entry);
let roughness = mat_roughness(entry);
// PBR shading with albedo + roughness + GI from cascade_atlas_0
```

This works because `MeshOutput.material_ids` already stores global MaterialIds per quad,
not palette indices. The palette is a per-chunk compression layer; the mesh is a
per-chunk surface layer; both ultimately index into the same global table.

---

## How I-3 Sets `has_emissive`

The summary rebuild pass scans the palette against `material_table` to derive bit 2 of
`chunk_flags`:

```wgsl
// I-3: summary rebuild for a dirty chunk
let palette_size = palette_meta[slot] & 0xFFFFu;
var is_emissive = false;
for (var i: u32 = 0u; i < palette_size; i = i + 1u) {
    let mat_id = (chunk_palette_buf[slot][i >> 1u] >> ((i & 1u) * 16u)) & 0xFFFFu;
    if mat_id == MATERIAL_EMPTY { continue; }
    let entry = material_table[mat_id];
    if mat_is_emissive(entry) {
        is_emissive = true;
        break;
    }
}
// Write result into chunk_flags[slot] bit 2
let flags = chunk_flags[slot] & ~HAS_EMISSIVE_BIT;
chunk_flags[slot] = flags | select(0u, HAS_EMISSIVE_BIT, is_emissive);
```

Average palette size is small (4–16 entries per chunk), so this scan costs negligible
memory bandwidth per dirty chunk.

---

## Material Table Invalidation

### On Material Property Change

When a material's properties are updated (e.g., emissive color changes):

1. CPU writes the new `MaterialEntry` to `material_table[material_id]` via `writeBuffer`
2. CPU sets `material_table_version` uniform (a u32 counter, incremented per change)
3. CPU sets `stale_summary` for **all** resident chunk slots (brute-force sweep)
4. The GPU compaction pass enqueues all dirty slots into `summary_rebuild_queue`
5. The I-3 summary rebuild re-scans palettes against the updated global table and
   re-derives `has_emissive` for each affected chunk

This O(N_SLOTS × avg_palette_size) scan is trivially cheap: 1024 slots × 8 entries ×
one `mat_is_emissive()` test = ~8192 integer comparisons per material change event.

Material property changes are not expected to happen per-frame. One global buffer write
+ brute-force sweep is the correct tradeoff at this scale.

### On Palette Change (Voxel Edit)

When a voxel edit changes which materials are referenced in a chunk (e.g., a new material
is painted or a material disappears from the chunk entirely):

1. Edit kernel updates `chunk_palette_buf[slot]`, `chunk_index_buf[slot]`, `palette_meta[slot]`
2. Edit kernel atomically increments `chunk_version[slot]`
3. Propagation pass sets `stale_summary[slot]` (because `has_emissive` may have changed)
4. Summary rebuild re-scans the palette on next I-3 pass

This is the normal chunk edit path — no special handling needed.

---

## Flat Color vs. Texture Atlas

**Flat color is the correct default.** All three material-reading stages confirm this:

- **R-6** (cascade GI): reads `emissive_rgb` only. Textures are irrelevant to radiance
  transport computation.
- **I-3** (summary): reads `has_emissive` boolean. Textures are irrelevant.
- **R-5** (color pass): reads `albedo_rgb` for surface shading. Flat color per material
  provides full per-voxel color variation when materials are assigned per-voxel by the
  producer.

Texture atlas is an optional upgrade for R-5 surface detail. When needed, it can be added
by extending `MaterialEntry` with an `atlas_tile_id: u16` field and having the greedy
mesher emit UV coordinates per quad. This is a strictly additive change and does not
affect the global table structure, the I-3 scan logic, or the R-6 emissive path.

**When atlas pays off:** stylized block-face textures, architectural visualization.
**When it doesn't:** physics/GI testbeds where visual richness comes from per-voxel
material assignment resolution rather than per-face texture detail.

---

## Global Table Buffer

```
material_table: array<MaterialEntry, MAX_MATERIALS>
    Binding:      read-only storage buffer in R-5, R-6, I-3
    Lifetime:     scene lifetime (re-uploaded on material change)
    Max entries:  4096 (expandable to 65536 if needed; u16 MaterialId allows it)
    Memory:       4096 × 16B = 64KB — fits in L2 cache on most GPUs
```

`MAX_MATERIALS = 4096` is the initial upper bound. For a testbed loading OBJ models with
material groups, the practical count is tens to low hundreds.

The table is a flat linear array. No indirection beyond `material_table[mat_id]`. All
passes that read material properties bind the same buffer object — no per-pass copies.

---

## Additions to the Per-Slot Layout

`palette_meta` is a new authoritative per-slot field added to the GPU chunk pool:

```
palette_meta[slot]    1 u32    palette_size (u16) + bits_per_entry (u8) + reserved (u8)
                               authoritative — CPU-written at chunk load and palette resize
```

This field belongs in the Data Plane alongside `chunk_palette_buf` and `chunk_index_buf`.
See [[gpu-chunk-pool]] — Per-slot layout.

---

## Producers

Any producer (voxelizer, procedural generator, OBJ importer) that writes chunk voxels must:

1. Assign MaterialIds from the global material table. If a material doesn't exist yet,
   register it (CPU-side, write to `material_table`) before chunk upload.
2. Write `chunk_palette_buf[slot]` with the global MaterialIds referenced in this chunk.
3. Write `chunk_index_buf[slot]` with bitpacked per-voxel palette indices.
4. Write `palette_meta[slot]` with the correct `palette_size` and `bits_per_entry`.
5. Write `chunk_occupancy_atlas[slot]` as usual.
6. Follow the standard edit-protocol signaling (`chunk_version`, `dirty_chunks`,
   `stale_summary` — see [[edit-protocol]]).

Producers must not embed material properties into `chunk_palette_buf`. The palette is an
index structure. Property data belongs in `material_table`.

---

## What Needs to Be Built

| Component | Status | Blocks |
|---|---|---|
| `MaterialEntry` struct (Rust + WGSL) | Not implemented | Everything |
| `material_table` GPU storage buffer | Not implemented | R-5, R-6, I-3 material reads |
| `palette_meta[slot]` per-slot field | Not implemented | R-6 index unpack, I-3 palette scan |
| I-3 summary rebuild — `has_emissive` scan against global table | Not implemented | `chunk_flags.has_emissive` |
| R-5 fragment shader — `material_table` binding + `mat_albedo()` lookup | Not implemented | Colored surface rendering |
| R-6 cascade compute — `material_table` binding + `mat_emissive()` at hit | Not implemented | Emissive GI |
| CPU material registration API | Not implemented | Producer integration |
| Material property change invalidation (sweep `stale_summary`) | Not implemented | Correct `has_emissive` on material edit |

Build order: `MaterialEntry` struct → `material_table` buffer → `palette_meta` field →
I-3 emissive scan → R-6 hit emissive → R-5 albedo.

---

## See Also

- [[chunk-field-registry]] — `materials.palette`, `materials.index_buf`, `chunk_flags.has_emissive`
- [[gpu-chunk-pool]] — per-slot layout; `chunk_palette_buf` and `chunk_index_buf` allocation
- [[pipeline-stages]] — R-5 (albedo read), R-6 (emissive hit), I-3 (palette scan)
- [[traversal-acceleration]] — material fetch is second-stage, never part of the DDA hot loop
- [[edit-protocol]] — `stale_summary` triggering and the brute-force invalidation path
- [[../greedy-meshing-docs/adr/0007-material-strategy]] — ADR-0007 `MaterialDef` (superseded by this spec)
