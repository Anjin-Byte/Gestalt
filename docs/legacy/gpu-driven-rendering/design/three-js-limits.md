# Three.js Limits — Evidence for Custom Pipeline

Date: March 9, 2026

---

## Purpose

This document catalogs specific technical limitations of Three.js that block
planned features. Each limitation references concrete code locations in the
Gestalt codebase and explains why the limitation cannot be worked around
within Three.js's public API.

---

## 1. Depth Buffer Is Not Application-Owned

### What the features need

Radiance cascades (ADR-0010), Hi-Z culling, and cluster culling all require
reading the depth buffer in a compute pass between the depth write and the
main color pass.

### What Three.js provides

The render call at `apps/web/src/viewer/threeBackend.ts:114`:

```typescript
renderer?.render(scene, camera);
```

This is atomic. Three.js's `WebGPURenderer` internally creates a depth
attachment, writes to it during the draw, and may discard it at pass end.
The depth texture is not returned to the application.

### Can it be worked around?

Three.js r167 (the version in `apps/web/package.json`) exposes
`renderer.backend.device` for diagnostic access
(`apps/web/src/viewer/webgpuDiagnostics.ts:12`). However:

1. The depth texture handle is not exposed on any public property.
2. `WebGPURenderer` uses an internal `RenderContext` that manages framebuffer
   attachments. Extracting the depth texture requires accessing private fields
   (`_renderContext`, `_textures`) whose layout is not part of the public API
   and changes between releases.
3. Even if extracted, the texture may be created with `GPUTextureUsage.RENDER_ATTACHMENT`
   only, lacking `TEXTURE_BINDING` — making it unsampleable in compute.

**Verdict:** Not possible through public API. Private-field access is fragile.

---

## 2. No Compute Pass Insertion Points

### What the features need

The frame pipeline requires multiple compute dispatches interleaved with render
passes:

```
depth prepass → [compute: Hi-Z pyramid] → [compute: cull] → color pass
                [compute: cascade build] → [compute: cascade merge] ↗
```

### What Three.js provides

`WebGPURenderer.render()` encodes a single render pass (or a small number of
internal passes for shadows/transmissive). There is no hook for:

1. Running compute shaders between the depth write and color draw
2. Splitting the render into explicit depth-only and color-only passes
3. Injecting GPU command encoder work between passes

The `onBeforeRender` / `onAfterRender` callbacks on `Object3D` run on the
CPU during traversal — they cannot insert GPU compute work.

### Can it be worked around?

One could call `renderer.render()` twice (once for depth, once for color)
with material overrides. But:

1. Three.js re-traverses the scene graph each call (CPU overhead)
2. The depth texture from the first pass cannot be accessed for compute
   (same ownership problem as §1)
3. The second pass cannot reuse the first pass's depth buffer without
   internal API manipulation

**Verdict:** Fundamental design mismatch. Three.js assumes a forward renderer
with a single render pass per frame.

---

## 3. No Indirect Draw

### What the features need

GPU-driven culling produces a visibility buffer on the GPU. The results must
feed back into draw calls without a CPU round-trip:

```wgsl
// GPU fills this buffer
struct DrawIndexedIndirectArgs {
    index_count: u32,
    instance_count: u32,  // 0 = culled, 1 = visible
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
};
```

The CPU issues `renderPass.drawIndexedIndirect(buffer, offset)` once per
frame. The GPU decides what to draw.

### What Three.js provides

Every `Mesh` object is drawn with `drawIndexed()` using CPU-side parameters
derived from `BufferGeometry.index.count` and `BufferGeometry.drawRange`. The
renderer iterates the scene graph on the CPU and issues one `drawIndexed` per
visible mesh.

There is no API to:
1. Submit an indirect draw buffer
2. Skip CPU-side scene graph traversal for a subset of objects
3. Batch multiple geometries into a single multi-draw call

### Can it be worked around?

Not within Three.js's draw loop. One could use `renderer.backend.device` to
create a separate `GPURenderPassEncoder` and issue indirect draws, but this
would bypass Three.js's material system, transform handling, and state
management entirely — at which point Three.js is no longer rendering those
objects.

**Verdict:** Requires replacing the draw loop for affected objects.

---

## 4. No G-Buffer / Multi-Render-Target Output

### What the features need

Radiance cascades and deferred-style lighting need additional per-pixel data
beyond color:

- Surface normal (for hemisphere integration)
- Material ID (for emissive lookup)
- World position (for probe lookup)

These are typically written as a G-buffer via multiple render targets (MRT)
in a single geometry pass.

### What Three.js provides

`WebGPURenderer` writes to a single color attachment plus depth. MRT output
requires either:

1. A custom `RenderTarget` with multiple color attachments — Three.js supports
   `WebGLMultipleRenderTargets` for WebGL but the WebGPU equivalent is
   limited and material shaders would need to write to multiple outputs.
2. `NodeMaterial` with custom outputs — possible in theory but tightly coupled
   to Three.js's node-based shader graph, which is experimental and has
   limited documentation for WebGPU MRT.

### Can it be worked around?

Partially, with significant effort. A `NodeMaterial` could potentially write
to MRT outputs, but:

1. The node material system is still marked experimental in Three.js r167
2. Binding compute passes to read these outputs is still blocked by §2
3. The material must be shared across all chunk meshes, limiting flexibility

**Verdict:** Theoretically possible but fragile and experimental.

---

## 5. No Per-Object Draw Range Control from GPU

### What the features need

Cluster/meshlet culling requires rendering subsets of a single geometry buffer.
For example, a chunk's index buffer might contain 6 sub-ranges (one per face
direction), and the GPU culling pass decides which sub-ranges to draw.

### What Three.js provides

`BufferGeometry.drawRange` is a single `{ start, count }` pair set from the
CPU. There is no way to:

1. Set multiple draw ranges per geometry
2. Have the GPU modify the draw range
3. Use multi-draw to batch sub-ranges

The `ChunkMeshPool` design (`docs/greedy-meshing-docs/threejs-buffer-management.md`)
uses `drawRange` for active vertex management but assumes CPU-side control.

### Can it be worked around?

One could split each chunk into N separate `Mesh` objects (one per cluster),
but this multiplies draw calls and scene graph overhead — the opposite of
GPU-driven rendering's goal of minimizing CPU work.

**Verdict:** Architectural mismatch with GPU-driven sub-object culling.

---

## 6. Summary: Feature Feasibility Under Three.js

| Feature | Feasible in Three.js? | Reason |
|---------|----------------------|--------|
| Depth prepass (app-owned) | No | Depth texture not exposed |
| Hi-Z pyramid build | No | Requires depth prepass + compute |
| Chunk-level occlusion cull | No | Requires indirect draw |
| Meshlet/cluster cull | No | Requires sub-object draw ranges from GPU |
| Radiance cascade compute | No | Requires compute between passes |
| GI fragment shading | Partial | Custom shader possible but blocked without cascade data |
| G-buffer output | Partial | Experimental MRT support only |
| Debug overlays, helpers | Yes | Standard Three.js usage |
| Camera controls | Yes | Standard Three.js usage |
| WebGL2 fallback | Yes | Standard Three.js usage |

The split is clear: **rendering pipeline features are blocked; utility features
work fine.** This motivates the hybrid approach: keep Three.js for what works,
build custom WebGPU for what doesn't.

---

## See Also

- [`pipeline-architecture.md`](pipeline-architecture.md) — what the custom pipeline looks like
- [`hybrid-transition.md`](hybrid-transition.md) — how to migrate incrementally
- [`../philosophy.md`](../philosophy.md) — why this convergence is not accidental
- [`../../culling/hiz-occlusion-culling-report.md`](../../culling/hiz-occlusion-culling-report.md) — depth buffer gap analysis (§4)
