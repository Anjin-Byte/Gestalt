# Layer Model: Three Products from One Voxel Truth

The renderer needs three distinct products, not one. They share a common source but answer different questions and must not be conflated.

---

## The Three Products

### Product 1 — World-Space Occupancy Structure
*Answers: what does this ray hit? Is this point occupied? What material is here?*

Used by:
- Amanatides & Woo traversal (GI, shadows, picking)
- Radiance cascade world-space interval queries
- Probe visibility testing
- Any query that does not filter by camera visibility

**Camera visibility is the wrong filter here.** A chunk can be fully occluded from the camera and still be a valid target for a light ray, a probe, or a shadow query. Filtering world-space traversal by the raster visibility set produces incorrect lighting.

### Product 2 — Surface Structure
*Answers: what geometry does the camera see this frame?*

Used by:
- Primary raster pass (triangle draw calls)
- Depth buffer production
- G-buffer / material attributes per pixel

This is greedy-meshed chunk geometry. Triangles are the right primitive here — they map directly to GPU rasterization and benefit from decades of hardware optimization. Moving to a full traversal renderer would eliminate triangles from this product, but that is a separate renderer, not an optimization.

### Product 3 — Camera-Visibility Structure
*Answers: which chunks / geometry are worth drawing this frame?*

Used by:
- Frustum culling
- Hi-Z occlusion culling
- Indirect draw argument generation

**This is not world-space truth.** It is a per-frame, per-camera filter over Product 2. Results from Product 3 must never feed into Product 1 queries. Doing so produces "mysterious lighting bugs" where occluded geometry stops casting light.

---

## The Shared Source

All three products derive from the same canonical structure: **GPU-resident chunk occupancy**.

```
                     ┌──────────────────────────────────┐
                     │   GPU-RESIDENT CHUNK OCCUPANCY   │
                     │                                  │
                     │  opaque_mask (per chunk)         │
                     │  materials   (per chunk)         │
                     │  chunk_flags (summary bits)      │
                     │  occupancy_summary (coarse grid) │
                     └──────────┬───────────────────────┘
                                │
            ┌───────────────────┼───────────────────┐
            ▼                   ▼                   ▼
  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
  │   PRODUCT 1     │ │   PRODUCT 2     │ │   PRODUCT 3     │
  │  World-space    │ │  Surface        │ │  Visibility     │
  │  occupancy      │ │  structure      │ │  structure      │
  │                 │ │                 │ │                 │
  │  chunk DDA      │ │  greedy mesh    │ │  frustum cull   │
  │  sub-brick DDA  │ │  vertex buffers │ │  Hi-Z pyramid   │
  │  voxel DDA      │ │  index buffers  │ │  indirect args  │
  └────────┬────────┘ └────────┬────────┘ └────────┬────────┘
           │                   │                   │
           ▼                   ▼                   ▼
  GI / probes / RC    primary raster pass   culled draw calls
  shadows / picking   depth buffer          (feeds Product 2
                      G-buffer              dispatch only)
```

Product 3 (visibility) feeds back only into Product 2 dispatch. It does not constrain Product 1 queries.

---

## The Canonical Structure in Detail

See [[chunk-contract]] for the full field-level specification. Summary:

**Authoritative (source of truth, never derived):**
- `opaque_mask` — binary voxel occupancy, per chunk
- `materials` — palette-compressed material assignments, per chunk
- `coord`, `data_version`

**Derived from authoritative:**
- `occupancy_summary` — coarse-scale traversal acceleration (bricklet grid or mip pyramid)
- `chunk_flags` — summary bits (`is_empty`, `has_emissive`, `is_resident`, ...)
- `aabb` — tight world-space bounds for culling
- `mesh` — greedy mesh output (Product 2 input)

---

## The Pipeline Spine

```
Producer phase
  OBJ / density field / edit / simulation
       │
       │  writes via CompactVoxel[] courier
       ▼
Canonical phase
  GPU-resident chunk occupancy
  (opaque_mask + materials + summaries)
       │
       ├──── derive ────► greedy mesh buffers  ──► Product 2
       │
       ├──── derive ────► occupancy hierarchy  ──► Product 1
       │
       └──── derive ────► Hi-Z pyramid         ──► Product 3
                          (from depth buffer,
                           after Product 2 draw)
```

CompactVoxel[] is a courier between the producer and canonical phases. It is not itself canonical. Once chunk occupancy is populated, the compact list is discardable.

---

## What Hi-Z Is and Is Not

Hi-Z is a **Product 3 tool**. It answers "which chunks or groups of triangles were occluded from this camera this frame?" That is a valid and useful question for reducing raster overhead.

Hi-Z is not:
- A world-space visibility oracle
- A prefilter for ray traversal
- A substitute for occupancy hierarchy

The occupancy hierarchy (chunk DDA → sub-brick DDA → voxel DDA) is the correct prefilter for Product 1. It skips regions the ray never enters, which is what Amanatides & Woo is optimized for. Hi-Z skips regions the *camera* doesn't see, which is a different and incompatible filter for that use case.

---

## Traversal Hierarchy for Product 1

A&W traversal over world-space chunk occupancy should proceed in three levels. See [[traversal-acceleration]] for full design.

```
Level 0 — Chunk DDA
  Step through the chunk grid using Amanatides & Woo
  Test chunk_flags.is_empty — skip entirely if true
  Cost: O(chunks crossed along ray)

Level 1 — Sub-brick DDA
  Within a non-empty chunk, step through the bricklet grid
  Test occupancy_summary bit for each bricklet — skip if empty
  Cost: O(bricklets crossed within chunk)

Level 2 — Voxel DDA
  Within a non-empty bricklet, step through per-voxel opaque_mask bits
  Cost: O(voxels crossed within bricklet)
```

Each level only descends when the coarser level confirms presence. This is the compound benefit: three nested A&W loops, each skipping empty space at its own resolution.

---

## Radiance Cascades and This Model

Sannikov's world-space probe variant uses a 3D regular grid of probes with voxelized world-space data. That maps directly to Product 1.

- Probes are placed in world space (not filtered by camera visibility)
- Each probe's raymarching queries use world-space occupancy
- The cascade merge pass operates on probe data, independent of the raster pass
- The final GI application reads merged cascade 0 in the fragment shader (Product 2 territory)

The hybrid screenspace variant (ADR-0010, Option C) places probes on the depth buffer surface (Product 2 output) but marches rays through world-space occupancy (Product 1). That is the correct use of the split — probe *placement* is screen-driven, probe *evaluation* is world-space.

See [[../greedy-meshing-docs/adr/0010-radiance-cascades]] for the full decision.

---

## See Also

- [[chunk-contract]] — canonical chunk field specification
- [[traversal-acceleration]] — multi-level DDA design
- [[pipeline-stages]] — GPU stage diagram with exact buffers and read/write ownership
- [[../gpu-driven-rendering/INDEX]] — ADR-0011 hybrid pipeline (Product 2 + 3 implementation)
