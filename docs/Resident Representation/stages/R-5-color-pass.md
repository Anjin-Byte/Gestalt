# Stage R-5: Main Color Pass

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Stage type:** GPU render (color + depth test)
**Trigger:** Every frame, after R-4 (occlusion cull).

> Draws surviving chunks via drawIndexedIndirect from indirect_draw_buf. Fragment shader resolves material via the global material table, applies albedo and lighting. Depth test against R-2 depth — no depth write needed since the depth buffer is already populated.

---

## Purpose

R-5 is the final rasterization stage that produces the visible color image. It draws only the geometry that survived occlusion culling (R-4), using indirect draw calls. The fragment shader resolves per-vertex material IDs against the global `material_table` to obtain albedo, roughness, and emissive properties. Lighting is applied in the fragment shader — currently placeholder ambient, with full GI integration via `cascade_atlas_0` deferred to R-8 wiring.

Because the depth buffer was fully populated by R-2, R-5 runs with depth test enabled but depth writes disabled. Every fragment that fails the depth test is rejected before its shader executes (early-Z), ensuring fragment shading cost is proportional to visible surface area.

---

## Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-1 | `indirect_draw_buf` contains valid draw entries for surviving geometry | R-4 postcondition (POST-1, POST-3) |
| PRE-2 | `draw_count` reflects the number of valid entries in `indirect_draw_buf` | R-4 postcondition (POST-3) |
| PRE-3 | `vertex_pool` contains valid vertex data referenced by indirect draw args | R-1 postcondition |
| PRE-4 | `index_pool` contains valid index data referenced by indirect draw args | R-1 postcondition |
| PRE-5 | `depth_texture` contains valid depth from R-2 | R-2 postcondition (POST-1) |
| PRE-6 | `material_table` is populated with valid material entries | Scene init / material registration |
| PRE-7 | `camera_uniform` contains current frame's view and projection matrices | App per-frame update |
| PRE-8 | R-4 has completed — `indirect_draw_buf` is fully written | Pipeline barrier between R-4 compute and R-5 indirect draw |

---

## Inputs

| Buffer / Texture | Access | Format | What's read |
|---|---|---|---|
| `vertex_pool` | Read (vertex fetch) | `array<f32>` (packed 16 B/vertex) | Position + packed normal + material ID per vertex |
| `index_pool` | Read (index fetch) | `array<u32>` | Triangle indices |
| `indirect_draw_buf` | Read (indirect) | `array<DrawIndexedIndirectArgs>` (20 B/entry) | Draw call arguments from R-4 |
| `draw_count` | Read | `u32` | Number of valid draw entries |
| `depth_texture` | Read (depth test) | `depth32float` | Depth comparison for early-Z rejection |
| `material_table` | Read (storage) | `array<MaterialEntry>` (16 B/entry) | Global material properties — albedo, roughness, emissive |
| `camera_uniform` | Read (uniform) | `mat4x4<f32>` x 2 + `vec3f` | `view_proj` for vertex transform; `camera_position` for specular |

---

## Transformation

### 1. Vertex Shader

Transforms vertex positions and passes material/normal data to the fragment shader:

```wgsl
struct VertexOutput {
    @builtin(position) clip_pos: vec4f,
    @location(0) world_pos:      vec3f,
    @location(1) world_normal:   vec3f,
    @location(2) @interpolate(flat) material_id: u32,
};

@vertex
fn vs_color(
    @location(0) position: vec3f,
    @location(1) normal_material: u32,
) -> VertexOutput {
    var out: VertexOutput;
    out.clip_pos = camera.view_proj * vec4f(position, 1.0);
    out.world_pos = position;

    // Unpack snorm8x3 normal
    let nx = f32(i32(normal_material & 0xFFu) << 24u >> 24u) / 127.0;
    let ny = f32(i32((normal_material >> 8u) & 0xFFu) << 24u >> 24u) / 127.0;
    let nz = f32(i32((normal_material >> 16u) & 0xFFu) << 24u >> 24u) / 127.0;
    out.world_normal = vec3f(nx, ny, nz);

    // Unpack material ID (u8 in bits 31:24)
    out.material_id = (normal_material >> 24u) & 0xFFu;

    return out;
}
```

### 2. Fragment Shader

Resolves material from the global table. Applies albedo and lighting:

```wgsl
@fragment
fn fs_color(in: VertexOutput) -> @location(0) vec4f {
    let entry = material_table[in.material_id];

    // Unpack material properties
    let albedo    = mat_albedo(entry);
    let roughness = mat_roughness(entry);
    let emissive  = mat_emissive(entry);

    // Placeholder ambient lighting (until cascade GI is wired via R-8)
    let ambient = vec3f(0.15);
    let n_dot_l = max(dot(in.world_normal, normalize(vec3f(0.3, 1.0, 0.2))), 0.0);
    let diffuse = albedo * (ambient + vec3f(n_dot_l * 0.85));

    // Emissive materials output emissive color directly
    let color = diffuse + emissive;

    return vec4f(color, 1.0);
}
```

When radiance cascades are wired (R-8):
- Replace `ambient` with hemisphere integration from `cascade_atlas_0` at the fragment's world position.
- Add cone query from `cascade_atlas_0` for specular reflection based on `roughness`.
- Emissive fragments continue to add `emissive` directly (they are light sources, not consumers of GI).

### 3. Material Resolution Path

The greedy mesher (R-1) resolves palette indices to global MaterialIds during mesh generation and packs the MaterialId into the vertex `normal_material` field (bits 31:24). The fragment shader does not perform palette lookup — it reads `material_table[material_id]` directly.

```
vertex.material_id → material_table[material_id] → MaterialEntry → albedo, roughness, emissive
```

This single indirection (one 16-byte cache-line read from the global table per fragment) is negligible against the rasterization and shading cost. The material table fits in L2 cache on most GPUs (4096 entries x 16 B = 64 KB).

### 4. Draw Submission

R-5 consumes the entire `indirect_draw_buf` via indirect draw calls:

```typescript
// Option A: loop over entries
for (let i = 0; i < draw_count; i++) {
    renderPass.drawIndexedIndirect(indirectDrawBuf, i * 20);
}

// Option B: multiDrawIndexedIndirect (when available)
renderPass.multiDrawIndexedIndirect(indirectDrawBuf, 0, draw_count);
```

Draw order within `indirect_draw_buf` is non-deterministic (depends on R-4 atomic ordering), but this does not affect correctness because R-2 has already populated the depth buffer. Early-Z rejects fragments regardless of draw order.

---

## Outputs

| Texture | Access | Format | What's written |
|---|---|---|---|
| `color_target` | Write (color attachment) | `rgba8unorm` | Final color output for this frame |

`depth_texture` is bound as the depth attachment for testing but with `depthWriteEnabled: false` — the depth buffer is not modified.

---

## Postconditions

| ID | Condition | Validates |
|---|---|---|
| POST-1 | `color_target` contains rendered color for all visible, unoccluded geometry | Visual correctness |
| POST-2 | `depth_texture` is unmodified (no depth writes) | DT-5 (R-5 does not write depth) |
| POST-3 | Every fragment's material_id resolved to a valid `MaterialEntry` | Material correctness |
| POST-4 | Emissive materials contributed their emissive color to the output | Emissive visibility |
| POST-5 | No fragment was drawn for geometry culled by R-4 | Cull correctness (implicit — only indirect_draw_buf entries are drawn) |

---

## Render Pass Configuration

```typescript
const colorPassPipeline = device.createRenderPipeline({
    vertex: {
        module: colorShaderModule,
        entryPoint: 'vs_color',
        buffers: [vertexBufferLayout],
    },
    fragment: {
        module: colorShaderModule,
        entryPoint: 'fs_color',
        targets: [{
            format: 'rgba8unorm',
        }],
    },
    primitive: {
        topology: 'triangle-list',
        frontFace: 'ccw',
        cullMode: 'back',
    },
    depthStencil: {
        format: 'depth32float',
        depthWriteEnabled: false,      // depth already populated by R-2
        depthCompare: 'less-equal',    // or 'greater-equal' for reversed-Z
    },
});
```

```typescript
const colorPassDescriptor: GPURenderPassDescriptor = {
    colorAttachments: [{
        view: colorTargetView,
        loadOp: 'clear',
        clearValue: { r: 0, g: 0, b: 0, a: 1 },
        storeOp: 'store',
    }],
    depthStencilAttachment: {
        view: depthTextureView,
        depthLoadOp: 'load',           // preserve R-2 depth
        depthStoreOp: 'store',         // keep for R-9 debug viz
    },
};
```

---

## Testing Strategy

### Unit tests (CPU-side)

1. **Pipeline configuration:** Verify `depthWriteEnabled: false` and `depthCompare: 'less-equal'`.
2. **Material unpack roundtrip:** Encode a `MaterialEntry` with known albedo/roughness/emissive, unpack via WGSL helper functions, verify exact match.
3. **Normal unpack roundtrip:** Encode axis-aligned normal into snorm8x3, unpack, verify decoded vector matches original.

### GPU validation

4. **Albedo correctness:** Assign a known albedo to a material, render a single quad, read back color target, verify the pixel color matches expected albedo under ambient lighting.
5. **Emissive output:** Assign a known emissive color, render, verify the output includes the emissive contribution (brighter than ambient-only).
6. **Depth test rejection:** Place two overlapping quads at different depths. Verify only the nearer quad's color appears in the output (farther quad rejected by depth test).
7. **No depth modification:** Read back `depth_texture` before and after R-5. Verify values are identical.
8. **Material boundary:** Render two adjacent quads with different materials. Verify the color boundary aligns with the geometry boundary.

### Integration tests

9. **R-4 -> R-5 pipeline:** Run full R-2 through R-5. Verify the color output matches a reference render for a known scene.
10. **Empty indirect buffer:** Set `draw_count = 0`. Verify R-5 produces a cleared color target with no rendered geometry.
11. **Fallback draw path:** Force meshlet staleness for one chunk. Verify R-5 correctly draws the chunk-level fallback entry from R-4.

---

## See Also

- [indirect-draw-buf](../data/indirect-draw-buf.md) — the draw argument buffer consumed by this stage
- [vertex-pool](../data/vertex-pool.md) — vertex data: position, normal, material ID packing
- [depth-texture](../data/depth-texture.md) — depth test source (R-2 output), not written by R-5
- [draw-metadata](../data/draw-metadata.md) — per-chunk draw parameters (consumed indirectly via indirect_draw_buf)
- [material-system](../material-system.md) — MaterialEntry layout, global table design, palette protocol
- [depth-prepass](../depth-prepass.md) — why R-5 can skip depth writes (R-2 already populated)
- [pipeline-stages](../pipeline-stages.md) — R-5 in the full stage diagram
