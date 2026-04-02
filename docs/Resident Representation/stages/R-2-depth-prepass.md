# Stage R-2: Depth Prepass

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Stage type:** GPU render (depth-only)
**Trigger:** Every frame. First render pass in the per-frame pipeline.

> Renders all resident non-empty chunks to depth_texture. Depth writes only — no fragment output. The prerequisite for the entire downstream pipeline.

---

## Purpose

The depth prepass populates the app-owned `depth_texture` with accurate depth values for all visible chunk geometry before any fragment shading runs. This serves three roles:

1. **Raster optimization:** The fully populated depth buffer lets the R-5 color pass reject occluded fragments via early-Z before their (expensive) fragment shaders execute.
2. **Hi-Z source:** R-3 builds the hierarchical depth pyramid from this texture for GPU-driven occlusion culling in R-4.
3. **Shared infrastructure:** R-6 (radiance cascade) reads depth to reconstruct probe world positions. R-9 (debug viz) reads depth for visualization and wireframe depth test.

The depth prepass is not just a performance optimization — it is the shared infrastructure prerequisite for the entire pipeline. It must be app-owned (not internal to Three.js) for downstream stages to read it.

---

## Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-1 | `vertex_pool` contains valid vertex data for all resident non-empty chunks | R-1 postcondition (mesh rebuild) |
| PRE-2 | `index_pool` contains valid index data for all resident non-empty chunks | R-1 postcondition |
| PRE-3 | `draw_metadata[slot]` is valid for all resident chunks with geometry | R-1 postcondition |
| PRE-4 | `camera_uniform` contains current frame's view and projection matrices | App per-frame update |
| PRE-5 | `depth_texture` exists at current viewport dimensions | App lifecycle (creation/resize handler) |
| PRE-6 | `chunk_flags` is readable for all slots | I-3 postcondition |

---

## Inputs

| Buffer / Texture | Access | Format | What's read |
|---|---|---|---|
| `vertex_pool` | Read (vertex fetch) | `array<f32>` (packed 16 B/vertex) | Position data for all chunk vertices |
| `index_pool` | Read (index fetch) | `array<u32>` | Triangle indices |
| `draw_metadata` | Read | `array<DrawMetadata>` (32 B/slot) | Per-chunk vertex_offset, index_offset, index_count for issuing draw calls |
| `camera_uniform` | Read (uniform) | `mat4x4<f32>` x 2 | `view` and `projection` matrices (or combined `view_proj`) |
| `chunk_flags` | Read | `array<u32>` | `is_empty` and `is_resident` bits to skip empty/unloaded chunks |
| `chunk_aabb` | Read | `array<vec4f>` (2 per slot) | Used for front-to-back sort ordering on CPU before draw submission |

---

## Transformation

### 1. Front-to-Back Sort (CPU, pre-dispatch)

Before issuing draw calls, the CPU sorts the draw list by chunk centroid distance from camera:

```
sort key = dot(chunk_center - camera_position, camera_forward)
```

This is O(N log N) over the active chunk count. Sorting front-to-back maximizes early-Z rejection: nearer geometry populates the depth buffer first, allowing the GPU's hardware early-Z unit to reject tiles of faraway geometry before rasterization.

### 2. Depth-Only Render Pass

A WebGPU render pass with no color attachments and a depth attachment:

```
GPURenderPassDescriptor {
    colorAttachments: [],
    depthStencilAttachment: {
        view:            depth_texture.createView(),
        depthLoadOp:     'clear',
        depthClearValue: 1.0,       // or 0.0 for reversed-Z
        depthStoreOp:    'store',
    },
}
```

### 3. Vertex Shader

The vertex shader performs only position transformation — no material lookup, no lighting, no varying interpolation beyond clip position:

```wgsl
@vertex
fn vs_depth(
    @location(0) position: vec3f,
    @location(1) normal_material: u32,   // unused in depth pass, but present in vertex layout
) -> @builtin(position) vec4f {
    return camera.view_proj * vec4f(position, 1.0);
}
```

No fragment shader is bound (depth-only write). The GPU writes depth from the rasterized vertex positions directly.

### 4. Draw Submission

For each resident, non-empty chunk (sorted front-to-back):

```
drawIndexed(
    indexCount:    draw_metadata[slot].index_count,
    instanceCount: 1,
    firstIndex:    draw_metadata[slot].index_offset,
    baseVertex:    draw_metadata[slot].vertex_offset,
    firstInstance: 0,
)
```

Chunks where `chunk_flags.is_empty == 1` or `chunk_flags.is_resident == 0` are skipped on the CPU side before draw call submission.

---

## Outputs

| Texture | Access | Format | What's written |
|---|---|---|---|
| `depth_texture` | Write (depth attachment) | `depth32float` | Full-viewport depth from all chunk geometry |

No other buffers or textures are written. The depth prepass has no color output.

---

## Postconditions

| ID | Condition | Validates |
|---|---|---|
| POST-1 | `depth_texture` contains valid depth for every visible surface pixel | DT-2 (depth completeness) |
| POST-2 | No color target was written | Depth-only pass contract |
| POST-3 | `depth_texture` format is `depth32float` with `RENDER_ATTACHMENT | TEXTURE_BINDING` usage | DT-1 (downstream readability) |
| POST-4 | Every resident non-empty chunk's geometry contributed to the depth buffer | No silent omissions |
| POST-5 | The depth buffer is fully populated before any downstream stage begins | DT-4 (pipeline ordering) |

---

## Render Pass Configuration

```typescript
// Depth-only pipeline (no fragment shader)
const depthPrepassPipeline = device.createRenderPipeline({
    vertex: {
        module: depthShaderModule,
        entryPoint: 'vs_depth',
        buffers: [vertexBufferLayout],   // matches vertex_pool packed layout
    },
    // No fragment stage — depth-only write
    primitive: {
        topology: 'triangle-list',
        frontFace: 'ccw',
        cullMode: 'back',               // backface culling enabled
    },
    depthStencil: {
        format: 'depth32float',
        depthWriteEnabled: true,
        depthCompare: 'less',            // or 'greater' for reversed-Z
    },
});
```

Backface culling is enabled — only front-facing triangles write depth. This halves rasterization work for closed geometry (which voxel chunk meshes are).

---

## Testing Strategy

### Unit tests (TypeScript, CPU-side)

1. **Pass configuration:** Verify render pass descriptor has zero color attachments, depth attachment with `loadOp: clear` and `storeOp: store`.
2. **Pipeline configuration:** Verify no fragment stage is bound. Verify `depthWriteEnabled: true` and `cullMode: 'back'`.
3. **Sort correctness:** For a known camera position and set of chunk centroids, verify the sort produces front-to-back order.

### GPU validation

4. **Depth readback:** Render a single known cube at a known distance. Read back `depth_texture` via staging buffer. Verify depth values at the cube's projected screen pixels match expected NDC depth.
5. **Clear value:** Render with no geometry. Verify all texels contain the clear depth value (1.0 standard, 0.0 reversed-Z).
6. **No color write:** Bind a color target alongside depth, render, verify color target is untouched (all clear color).
7. **Front-to-back benefit:** Render a scene sorted front-to-back and back-to-front, compare GPU timestamp queries — front-to-back should be measurably faster due to early-Z rejection.

### Cross-stage tests

8. **R-2 -> R-3:** After R-2, verify R-3 can successfully read `depth_texture` and build the Hi-Z pyramid.
9. **R-2 -> R-5:** After R-2, verify R-5 color pass with `depthWriteEnabled: false` produces correct results — fragments behind depth are rejected, fragments at depth pass.
10. **R-2 -> R-6:** After R-2, verify R-6 cascade build can reconstruct world positions from depth values via inverse projection.

---

## See Also

- [depth-texture](../data/depth-texture.md) — the texture written by this stage
- [vertex-pool](../data/vertex-pool.md) — vertex data consumed by vertex fetch
- [draw-metadata](../data/draw-metadata.md) — per-chunk draw call parameters
- [chunk-flags](../data/chunk-flags.md) — `is_empty` and `is_resident` bits for skip decisions
- [chunk-aabb](../data/chunk-aabb.md) — used for front-to-back sort ordering
- [hiz-pyramid](../data/hiz-pyramid.md) — built from depth_texture by R-3 (immediate downstream)
- [depth-prepass](../depth-prepass.md) — narrative design rationale for the raster optimization chain
- [pipeline-stages](../pipeline-stages.md) — R-2 in the full stage diagram
