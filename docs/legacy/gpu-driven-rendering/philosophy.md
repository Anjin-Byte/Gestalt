# GPU-Driven Rendering — Philosophy

**Type:** legacy
**Status:** legacy
**Date:** 2026-03-09

---

## The Convergence Problem

Three separate features — radiance cascades, Hi-Z occlusion culling, and
fine-grained mesh culling — were designed independently. Each appeared to have
its own technical requirements. But when their implementation prerequisites are
laid side by side, they converge on the same architectural gap:

| Feature | Needs depth prepass | Needs compute between passes | Needs indirect draw | Needs custom fragment |
|---------|-------------------|----------------------------|--------------------|--------------------|
| Radiance cascades (ADR-0010) | Yes | Yes (cascade build + merge) | No (Phase 1) | Yes (GI application) |
| Hi-Z occlusion culling | Yes | Yes (pyramid build + cull) | Yes (draw compaction) | No |
| Meshlet/cluster culling | Yes | Yes (per-cluster test) | Yes (visibility buffer) | Yes (deferred resolve) |

Every row requires an explicit depth target owned by the application, compute
shader dispatch between render stages, and — in two of three cases — the ability
to issue draw calls from GPU-generated command buffers. Three.js provides none
of these.

This is not a coincidence. These features are all instances of the same
architectural pattern: **GPU-driven rendering**, where the GPU makes per-object
or per-cluster visibility decisions and the CPU submits a single indirect draw
covering the entire scene. The depth prepass, the compute culling, and the
indirect draw are the three legs of this pattern. You cannot implement one leg
cleanly without at least acknowledging the other two.

---

## Why Not Bolt It Onto Three.js

The natural instinct is to reach into Three.js's internals and extract the
depth buffer after the render call. This fails for three reasons:

1. **Pass ordering.** Three.js decides internally when to write depth, when to
   resolve MSAA, and when to release transient attachments. Inserting a compute
   pass "between" the depth write and the color pass requires either
   reimplementing Three.js's render loop or depending on undocumented internal
   state that changes across versions.

2. **Resource ownership.** The depth texture must outlive the render pass that
   writes it so that subsequent compute passes can sample it. Three.js's
   WebGPURenderer treats the depth attachment as transient — it may not even
   exist as a sampleable texture. Forcing it to persist requires patching the
   renderer's framebuffer management.

3. **Draw submission.** Indirect draw (`drawIndexedIndirect`) is the mechanism
   by which GPU culling results feed back into rendering without a CPU
   round-trip. Three.js has no indirect draw path. Every mesh object goes
   through `drawIndexed` with CPU-determined parameters. Adding indirect draw
   means replacing the draw loop itself.

Each of these can be hacked individually. But hacking all three simultaneously
while maintaining Three.js compatibility across updates amounts to writing a
custom renderer anyway — just one that is coupled to Three.js's internal
implementation details rather than its public API.

---

## The Hybrid Principle

The alternative is to accept that the voxel chunk rendering path has outgrown
Three.js's abstraction and build a custom WebGPU pipeline for it, while
keeping Three.js for everything it does well.

**What Three.js keeps:**
- Scene graph for debug objects (grid, axes, bounding boxes, sprites)
- Camera management and controls (orbit, free camera)
- Standard materials for non-chunk objects
- WebGL2 fallback for environments without WebGPU

**What the custom pipeline owns:**
- Depth prepass for chunk meshes
- Hi-Z pyramid build
- Occlusion and cluster culling
- Radiance cascade build and merge
- Indirect draw dispatch for visible chunks
- GI-aware fragment shading
- Final composite with Three.js overlay

This split mirrors a pattern common in production engines: a high-performance
"world renderer" for the dominant geometry (chunks), and a general-purpose
"utility renderer" for everything else (debug, UI, editor gizmos). The two
share a camera and a final render target but otherwise operate independently.

---

## Relationship to Existing Architecture

This philosophy does not invalidate prior decisions. It refines them:

- **ADR-0001 (Renderer Choice)** chose Three.js for rapid prototyping and
  backend flexibility. That choice was correct for the project's early phase.
  ADR-0011 amends it: Three.js remains for utility rendering, but the
  performance-critical chunk path moves to a custom pipeline.

- **ADR-0003 (Binary Greedy Meshing)** and **ADR-0004 (64³ Chunks)** define the
  data that the custom pipeline consumes. Nothing about the meshing algorithm
  or chunk layout changes.

- **ADR-0007 (Material Strategy)** defines material properties that the custom
  pipeline's fragment shader will use. The `MaterialRegistry` and atlas system
  remain valid; only the shader that reads them changes.

- **ADR-0009 (GPU-Compact Voxelizer Integration)** feeds data into the chunk
  manager, which feeds the custom pipeline. The data flow is unchanged.

- **ADR-0010 (Radiance Cascades)** is a consumer of the custom pipeline's depth
  prepass and a producer of GI data for the custom pipeline's fragment shader.
  Its architecture was designed anticipating this split.

The custom pipeline is the infrastructure that makes the next generation of
features possible. It is not a replacement for the entire application — it is
the rendering backend for the application's dominant workload.

---

## See Also

- [`adr/0011-hybrid-gpu-driven.md`](adr/0011-hybrid-gpu-driven.md) — formal decision record
- [`design/three-js-limits.md`](design/three-js-limits.md) — detailed evidence for the Three.js gap
- [`design/pipeline-architecture.md`](design/pipeline-architecture.md) — target frame pipeline
- [`../../adr/0010-radiance-cascades.md`](../../adr/0010-radiance-cascades.md) — radiance cascades ADR
- [`../culling/hiz-occlusion-culling-report.md`](../culling/hiz-occlusion-culling-report.md) — Hi-Z readiness report
