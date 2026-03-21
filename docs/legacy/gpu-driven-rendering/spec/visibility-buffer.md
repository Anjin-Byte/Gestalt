# Visibility Buffer and Meshlet Design

Date: March 9, 2026
Status: **Future** — not required for initial pipeline. Documents the target
state for Stage 3-4 of the culling granularity progression.

---

## Context

The initial custom pipeline (Phases 1-4 of the hybrid transition) uses
chunk-level culling: one AABB per chunk, one indirect draw per chunk. This is
effective for large occluders but coarse for chunks with sparse interiors or
partially visible geometry.

This document specifies the finer-grained culling system that extends the
pipeline to sub-chunk clusters (meshlets) and optionally to a full visibility
buffer.

---

## Meshlet / Cluster Definition

A **cluster** is a contiguous group of triangles within a chunk's index buffer,
with an associated AABB. The cluster is the atomic unit of GPU culling.

### Generation

The greedy mesher already processes faces per axis direction (±X, ±Y, ±Z).
This provides a natural first-level clustering:

```
Chunk index buffer layout:
  [+X faces | -X faces | +Y faces | -Y faces | +Z faces | -Z faces]
   ▲ cluster 0          ▲ cluster 2            ▲ cluster 4
             ▲ cluster 1            ▲ cluster 3            ▲ cluster 5
```

Each face-direction group is a cluster with its own:
- Index range: `(first_index, index_count)` within the chunk's index buffer
- AABB: tight bounding box of the triangles in that group
- Normal direction: known at generation time (used for backface culling)

**Why face-direction clusters are effective:**

1. **Free backface culling.** For any camera orientation, 3 of 6 face
   directions point away from the camera. These clusters can be rejected
   with a single dot product — no Hi-Z test needed.

2. **Tighter AABBs.** A chunk's +Y faces may occupy only the top surface.
   The +Y cluster's AABB is much smaller than the full chunk AABB,
   enabling tighter occlusion rejection.

3. **Zero cost to generate.** The greedy mesher already groups faces by
   direction. Outputting the group boundaries as metadata requires only
   recording 6 offsets.

### Future: Sub-Direction Clusters

For very large chunks or high triangle counts, face-direction groups can be
further subdivided into meshlets of 64-128 triangles. Standard meshlet
generation (greedy spatial clustering) applies. This is not needed initially
but the data layout supports it.

---

## Cluster Metadata

```rust
/// Per-cluster data, stored in a GPU buffer
struct ClusterMeta {
    /// Offset into the global index buffer
    first_index: u32,
    /// Number of indices in this cluster
    index_count: u32,
    /// Chunk ID this cluster belongs to (for transform lookup)
    chunk_id: u32,
    /// Padding / flags (backface direction encoded in 3 bits)
    flags: u32,
    /// AABB center in world space
    center: [f32; 3],
    /// AABB half-extents
    extents: [f32; 3],
    /// Dominant normal direction (for backface test)
    normal: [f32; 3],
    _pad: f32,
}
// Size: 64 bytes per cluster (cache-line aligned)
```

For 4096 chunks × 6 clusters = 24,576 clusters:
- Metadata buffer: 24,576 × 64 = **1.5 MB**
- Indirect args buffer: 24,576 × 20 = **480 KB** (`DrawIndexedIndirectArgs`)

---

## Cluster Culling Pass

Replaces Pass 3 from `frame-graph.md` when cluster culling is enabled:

```wgsl
@compute @workgroup_size(64)
fn cull_clusters(
    @builtin(global_invocation_id) gid: vec3u,
) {
    let cluster_id = gid.x;
    if cluster_id >= cluster_count { return; }

    let meta = cluster_meta[cluster_id];
    var visible = true;

    // 1. Backface test (free — just a dot product)
    let view_dir = normalize(camera_position - meta.center);
    if dot(view_dir, meta.normal) < -0.1 {
        visible = false;
    }

    // 2. Frustum test (cheap — AABB vs 6 planes)
    if visible {
        visible = aabb_in_frustum(meta.center, meta.extents, frustum_planes);
    }

    // 3. Hi-Z occlusion test (if passed frustum)
    if visible {
        let screen_rect = project_aabb(meta.center, meta.extents, view_proj);
        let mip = select_mip(screen_rect);
        let pyramid_depth = sample_pyramid(depth_pyramid, screen_rect, mip);
        let aabb_near_depth = compute_near_depth(meta.center, meta.extents, view);

        if aabb_near_depth > pyramid_depth {
            visible = false;  // Fully behind existing depth
        }
    }

    // Write indirect args
    indirect_args[cluster_id].index_count = select(0u, meta.index_count, visible);
    indirect_args[cluster_id].instance_count = select(0u, 1u, visible);
    indirect_args[cluster_id].first_index = meta.first_index;
    indirect_args[cluster_id].base_vertex = 0;
    indirect_args[cluster_id].first_instance = cluster_id;  // For cluster ID in shader
}
```

---

## Visibility Buffer Rendering (Stage 4, Optional)

A visibility buffer replaces the traditional "shade all fragments" approach
with "identify which triangle is visible per pixel, then shade only visible
pixels."

### Pass 1 (Modified): Visibility Write

Instead of writing depth only, the depth prepass writes a packed ID per pixel:

```wgsl
struct VisOutput {
    @builtin(position) pos: vec4f,
    @location(0) vis_id: u32,  // (cluster_id << 8) | triangle_id_in_cluster
}

@fragment
fn vis_fragment(in: VisOutput) -> @location(0) u32 {
    return in.vis_id;
}
```

Output: `visibility_texture` (r32uint) + `depth_texture`

### Pass 5 (Modified): Deferred Resolve

A fullscreen compute or fragment shader reads `visibility_texture`, recovers
the triangle, interpolates attributes, and shades:

```wgsl
@fragment
fn resolve(in: FullscreenVertex) -> @location(0) vec4f {
    let vis_id = textureLoad(visibility_texture, pixel, 0).r;
    if vis_id == 0xFFFFFFFF { return background_color; }

    let cluster_id = vis_id >> 8;
    let tri_id = vis_id & 0xFF;

    // Recover triangle vertices from cluster's index/vertex buffers
    let tri = load_triangle(cluster_id, tri_id);

    // Compute barycentrics from pixel position
    let bary = compute_barycentrics(tri, pixel);

    // Interpolate attributes
    let normal = interpolate(tri.normals, bary);
    let uv = interpolate(tri.uvs, bary);
    let material_id = tri.material_id;

    // Shade (same as current fragment shader)
    let albedo = sample_material_atlas(uv, material_id);
    let gi = query_cascade_atlas(world_pos, normal);
    return shade(albedo, normal, gi, lights);
}
```

### Why This Is Optional

The visibility buffer eliminates overdraw entirely — each pixel is shaded
exactly once. But:

1. The attribute recovery (loading triangle vertices, computing barycentrics)
   has a per-pixel cost that can exceed the saved overdraw cost in simple scenes.
2. Implementation complexity is high (attribute buffer management, barycentric
   computation, edge cases at triangle boundaries).
3. Chunk-level and cluster-level culling already eliminate most overdraw for
   voxel scenes (greedy-meshed chunks have very low overdraw by construction).

**Recommendation:** Implement only if profiling shows overdraw is a bottleneck
after Stage 2-3 culling is in place.

---

## Integration with Existing Mesher

The greedy mesher (`crates/greedy_mesher`) must output cluster metadata alongside
mesh data. Required changes:

### Rust Side

```rust
/// Added to MeshOutput
pub struct MeshOutput {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
    pub uvs: Vec<f32>,
    pub material_ids: Vec<u16>,

    // NEW: cluster metadata
    pub cluster_offsets: Vec<ClusterOffset>,  // 6 entries (one per face direction)
}

pub struct ClusterOffset {
    pub first_index: u32,
    pub index_count: u32,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
    pub normal_direction: u8,  // 0=+X, 1=-X, 2=+Y, 3=-Y, 4=+Z, 5=-Z
}
```

### WASM Bindings

```rust
#[wasm_bindgen]
impl MeshResult {
    // Existing
    pub fn positions(&self) -> Vec<f32>;
    pub fn normals(&self) -> Vec<f32>;
    pub fn indices(&self) -> Vec<u32>;

    // NEW
    pub fn cluster_count(&self) -> u32;
    pub fn cluster_first_index(&self, i: u32) -> u32;
    pub fn cluster_index_count(&self, i: u32) -> u32;
    pub fn cluster_aabb_min(&self, i: u32) -> Vec<f32>;
    pub fn cluster_aabb_max(&self, i: u32) -> Vec<f32>;
    pub fn cluster_normal_direction(&self, i: u32) -> u8;
}
```

### Impact on Existing Code

The greedy mesher's `expand.rs` already produces indices grouped by face
direction. The only change is recording the group boundaries (6 `u32` offset
pairs) and computing per-group AABBs during expansion. This is additive —
no existing output format changes.

---

## See Also

- [`frame-graph.md`](frame-graph.md) — pass ordering (this extends Pass 3 and optionally Pass 1/5)
- [`../design/pipeline-architecture.md`](../design/pipeline-architecture.md) — culling granularity progression
- [`../../greedy-meshing-docs/greedy-mesh-implementation-plan.md`](../../greedy-meshing-docs/greedy-mesh-implementation-plan.md) — mesher output format
- [`../../greedy-meshing-docs/binary-greedy-meshing-analysis.md`](../../greedy-meshing-docs/binary-greedy-meshing-analysis.md) — face-direction grouping in algorithm
