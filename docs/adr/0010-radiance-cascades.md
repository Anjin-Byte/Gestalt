# ADR-0010: Radiance Cascades for Global Illumination

Date: March 9, 2026
Status: **Proposed**
Depends on: ADR-0001 (renderer choice), ADR-0003 (binary greedy meshing), ADR-0007 (material strategy), ADR-0009 (GPU-compact voxelizer integration)
Implemented by: ADR-0011 (Hybrid GPU-Driven Pipeline) — see [`docs/gpu-driven-rendering/`](../../gpu-driven-rendering/INDEX.md)

---

## Context

The current rendering pipeline produces unlit or directly-lit voxel geometry. There is no global illumination: no indirect lighting, no color bleeding, no contact shadows from area emitters. The material system (ADR-0007) defines `emissive` properties per material, but nothing consumes them for light propagation.

Adding GI to a voxel engine is a well-studied problem. The dominant approaches are:

1. **Voxel Cone Tracing (VCT)** — voxelize scene, build mipmaps, trace cones
2. **Light Propagation Volumes (LPV)** — spherical harmonics on a grid, iterative flood
3. **Screen-space ray-traced probes (Lumen-style)** — screenspace traces, temporal accumulation
4. **Radiance Cascades** (Sannikov 2024) — hierarchical radiance interval decomposition

All four require a scene representation for ray queries. Gestalt already has one: the GPU voxelizer produces occupancy grids and the greedy mesher maintains `BinaryChunk` opaque masks — both are acceleration structures for raymarching.

### Why Radiance Cascades

Radiance cascades have several properties that make them uniquely suited to this project:

1. **Scene-independent cost.** Computation time is fixed regardless of scene complexity, number of light sources, or polygon count. This matters for a testbed where users load arbitrary models.

2. **No temporal dependency.** Each frame's radiance field is built from scratch — no ghosting, no temporal lag when lights or geometry change. This aligns with Gestalt's real-time voxel editing workflow.

3. **Asymptotic scaling.** Adding cascade *N* costs half of cascade *N-1* while doubling the effective ray count. Total cost for N cascades < 2× cost of cascade 0 alone (Eq. 21 of the paper). This means the "infinite ray" limit is achievable in finite budget.

4. **Natural voxel fit.** The paper's Radiance 3d implementation (Section 4.3) uses 3D voxelization as its scene representation — exactly what Gestalt already produces.

5. **Penumbra quality.** Radiance cascades degrade gracefully: reduced precision produces softer shadows (as if from area lights), not noisy artifacts. This is visually superior to path-tracing denoisers at low sample counts.

### The Variant Question

The paper describes several implementation variants:

| Variant | Probes | Raymarching | Memory | Speed | Off-screen light |
|---------|--------|-------------|--------|-------|-----------------|
| **Flatland (2D)** | 2D grid | 2D screen | O(screen) | ~12ms | No |
| **PoE2 screenspace** | On depth buffer | Screenspace | O(screen) | ~3ms | No |
| **World-space 3D** | 3D volume | Voxel/SDF | O(scene³) | ~30-50ms | Yes |
| **Hybrid (Section 4.5)** | On depth buffer | World-space voxel | O(screen) | ~30-50ms | Yes |

---

## Options Evaluated

### Option A: Pure Screenspace (PoE2 Style)

Probes placed on depth buffer pixels, raymarching done in screenspace (depth buffer lookups only).

**Pros:**
- Fastest variant (~3ms on mid-range GPU)
- Memory proportional to screen resolution only
- Proven in production (Path of Exile 2)

**Cons:**
- No off-screen light sources — light behind the camera is invisible
- No invisible occluders — geometry behind depth buffer doesn't cast shadows
- Requires explicit depth buffer ownership, which Gestalt does not have (see Hi-Z culling report: depth is internal to Three.js renderer draw)
- PoE2's fixed camera makes screenspace artifacts tolerable; Gestalt has a free camera where these limitations are more visible
- Screenspace raymarching requires a Hi-Z pyramid for efficient stepping — same prerequisite as the culling pipeline, but neither exists yet

**Verdict:** Fast but limited. Wastes the voxel data Gestalt already has.

### Option B: Full World-Space 3D

Probes on a regular 3D grid, raymarching through the voxel occupancy volume.

**Pros:**
- Captures all lighting phenomena (off-screen, through walls, atmospheric)
- Voxel occupancy grid is the acceleration structure — no additional data needed
- Enables volumetric effects (fog, scattering) in the future

**Cons:**
- Cascade 0 memory = scene_size³ × probe_data. For a 256³ world with 8×8 octahedral probes per probe position: 256³ positions × 64 texels × 4 bytes = ~4 GB. Impractical.
- Even with the cascade sum converging to 2× cascade 0, the base cost is the bottleneck
- The paper explicitly calls this "often a dealbreaker for large-scale scenes" (p. 29)
- Raymarching a full 3D volume per probe per direction is expensive (~30-50ms) even with radiance interval extension (Eq. 16)

**Verdict:** Theoretically ideal but memory-prohibitive at meaningful resolutions.

### Option C: Hybrid — Screenspace Probes with World-Space Radiance Intervals

Probes placed on the depth buffer surface (like screenspace), but rays march through the world-space voxel occupancy grid rather than the depth buffer.

This is the approach described in Section 4.5 of the paper. Probes of cascade *i* are octahedral maps of size ~2^i × 2^i texels, placed 2^i pixels apart on the depth buffer. Spatial interpolation uses bilateral filtering (depth-aware) to prevent light leaks. Directional interpolation uses standard bilinear.

**Pros:**
- **Memory is screen-proportional**, not scene-proportional — same budget as pure screenspace
- **Captures off-screen light** — rays march through world-space voxels, so emissive geometry behind the camera or behind walls contributes correctly
- **Gestalt already has the hard part** — the voxelizer produces occupancy grids that serve as the raymarching acceleration structure
- **Depth buffer probe placement** is simpler than a full 3D probe volume — one 2D atlas per cascade
- **Radiance interval extension** (Eq. 16) keeps per-probe raymarching cheap: march a short range, then iteratively double via merging
- Surface-only probes naturally align with the greedy-meshed chunk geometry where lighting will be applied

**Cons:**
- Still requires depth buffer access between passes (same as Option A), shared prerequisite with Hi-Z culling pipeline
- World-space raymarching is slower than screenspace raymarching (~30-50ms vs ~3ms per the paper), though temporal accumulation amortizes this
- Bilateral interpolation adds complexity vs. simple bilinear
- No volumetric effects (probes only exist on surfaces, not in empty space)

**Verdict:** Best quality-per-byte for a voxel engine with an existing occupancy grid.

---

## Decision

**Option C: Hybrid screenspace probes with world-space voxel raymarching.**

This variant maximizes the return on Gestalt's existing voxelizer investment while keeping memory bounded by screen resolution. The world-space raymarching eliminates the screenspace-only artifacts that would be unacceptable with Gestalt's free camera.

---

## Rationale

### The voxel grid is a free acceleration structure

The `BinaryChunk.opaque_mask` (`[u64; CS_P²]`) is a bitpacked 3D occupancy volume. Raymarching through it is a simple DDA loop with bit tests — no BVH, no SDF generation, no additional data structures. The chunk manager already maintains this data for meshing; radiance cascades consume it read-only.

### Screen-proportional memory is the only viable budget

For a 1080p screen with 4 cascades:
- Cascade 0: probes every 1px, 1×1 octahedral map → same as screen resolution × 4 bytes = ~8 MB
- Cascade 1: probes every 2px, 2×2 octahedral map → same memory as cascade 0 / 2
- Cascade 2: probes every 4px, 4×4 octahedral map → cascade 0 / 4
- Cascade 3: probes every 8px, 8×8 octahedral map → cascade 0 / 8
- Total: < 2 × cascade 0 ≈ **~16 MB**

Compare to full 3D at 256³: **~4 GB**. The hybrid is 250× cheaper.

### Depth buffer access is a shared prerequisite

Both this ADR and the Hi-Z occlusion culling pipeline (see `docs/culling/hiz-occlusion-culling-report.md`) require the same architectural change: explicit depth target ownership in `threeBackend.ts` rather than the current internal renderer draw. Implementing one unblocks the other. The recommended approach is:

1. Add a depth prepass that writes to an app-owned `GPUTexture`
2. The main color pass reuses this depth buffer (depth test, no depth write)
3. Both Hi-Z pyramid build and radiance cascade probe placement read from this texture

### Temporal accumulation makes 30-50ms tolerable

The paper reports 30-50ms per frame for world-space raymarching on a GTX3060. With temporal reprojection (each cascade reprojected from the previous frame), convergence takes 0.1-0.5s but per-frame cost drops significantly since only a fraction of probes need fresh rays each frame. For a testbed application, this tradeoff is acceptable.

---

## Architecture

### Cascade Data Structure

Each cascade *i* is stored as a 2D texture atlas:

```
Cascade i:
  - Probe grid:    (screen_w / 2^i) × (screen_h / 2^i) probes
  - Per-probe:     octahedral map of (2^i × 2^i) texels
  - Per-texel:     RGBA16F (radiance RGB + transparency alpha)
  - Radiance interval: [2^i, 2^(i+1)] in voxel units
  - Atlas size:    screen_w × screen_h × 4 × 2 bytes (RGBA16F)
```

All cascades tile into a single atlas texture of width `2 × screen_w` (cascades packed left to right, each half the probe count but double the per-probe resolution, totaling the same pixel area per cascade).

### Pipeline Overview

```
Per frame:
  1. Depth prepass           → depth_texture (app-owned GPUTexture)
  2. G-buffer pass           → normal + material_id + emissive
  3. Radiance cascade build  → compute shader per cascade (highest to lowest)
     a. For each probe position (from depth buffer):
        - Reconstruct world position from depth + screen UV
        - For each direction in octahedral map:
          - Raymarch voxel occupancy grid for interval [t_start, t_end]
          - Record radiance (from emissive hits) + transparency
     b. Temporal reprojection: blend with previous frame's cascade
  4. Cascade merge           → compute shader, back-to-front (N-1 → 0)
     a. For each probe in cascade i:
        - Interpolate cascade i+1 at this probe's position (bilinear)
        - Merge: L_merged = L_i + β_i × L_(i+1)   [Eq. 13]
  5. Lighting application    → fragment shader or compute
     a. For each fragment:
        - Query merged cascade 0 at fragment world position
        - Integrate hemisphere for diffuse (few directions, cosine-weighted)
        - Cone query along reflection for specular (roughness → cone angle)
     b. Combine: final_color = direct_light + indirect_diffuse + indirect_specular
  6. Main color pass         → standard Three.js render with GI contribution
```

### Voxel Raymarching

The raymarch kernel operates on the chunk manager's `opaque_mask` data, uploaded as a 3D texture or storage buffer:

```wgsl
// Pseudocode for voxel DDA raymarch
fn raymarch_voxels(
    origin: vec3f,
    direction: vec3f,
    t_start: f32,
    t_end: f32,
) -> RadianceInterval {
    var pos = origin + direction * t_start;
    var t = t_start;
    var radiance = vec3f(0.0);
    var transparency = 1.0;

    // DDA stepping through voxel grid
    let step = sign(direction);
    let t_delta = abs(1.0 / direction);
    var t_max = (floor(pos) + max(step, vec3f(0.0)) - pos) / direction;

    while t < t_end && transparency > 0.01 {
        let voxel = vec3i(floor(pos));

        // Look up chunk, then bit in opaque_mask
        if is_occupied(voxel) {
            let mat = get_material(voxel);
            let emissive = material_emissive(mat);
            radiance += transparency * emissive;
            transparency = 0.0;  // Opaque approximation
            break;
        }

        // Advance to next voxel boundary
        let axis = min_component_index(t_max);
        t = t_max[axis];
        t_max[axis] += t_delta[axis];
        pos = origin + direction * t;
    }

    return RadianceInterval(radiance, transparency);
}
```

### Integration with Existing Systems

| System | Current role | Radiance cascades role |
|--------|-------------|----------------------|
| `BinaryChunk.opaque_mask` | Meshing input | Raymarching acceleration structure (read-only) |
| `MaterialDef.emissive` (ADR-0007) | Unused | Light source definition — emissive voxels drive GI |
| `threeBackend.ts` | Renderer wrapper | Depth prepass host, cascade compute dispatch |
| `ChunkManager` | Voxel storage + dirty tracking | Provides occupancy data to GPU each frame |
| Module system | Testbed lifecycle | New `RadianceCascadesModule` or integrated into mesher module |
| Hi-Z culling pipeline | Not yet implemented | Shares depth prepass; cascade build runs after pyramid build |

### Occupancy Data Upload

The chunk manager's opaque masks must be available to the raymarch shader as a GPU-resident 3D structure. Two options:

1. **3D texture**: Pack `opaque_mask` bits into a 3D `r32uint` texture. Each texel stores one u32 (half a column). Texture sampling provides hardware-accelerated neighbor lookups.

2. **Storage buffer**: Upload raw `[u64; CS_P²]` per chunk as a flat SSBO. Requires manual addressing but preserves the 64-bit column layout. WGSL lacks u64, so each column becomes two u32 words.

Recommendation: **3D texture** for cascade raymarching (hardware filtering benefits bilinear probe interpolation), **storage buffer** for the meshing path (already u64-native). Both read the same source data.

---

## Implementation Plan

### Phase 1: Depth Prepass Infrastructure

**Prerequisite for both radiance cascades and Hi-Z culling.**

1. Add app-owned `GPUTexture` for depth in `threeBackend.ts`
2. Implement depth-only render pass before the main color pass
3. Expose depth texture to compute shader bindings
4. Verify depth is readable in a trivial compute shader (debug visualization)

**Deliverable:** Depth texture accessible to compute pipelines. Unblocks both this ADR and the culling pipeline.

### Phase 2: Single Cascade Prototype

1. Implement cascade 0 only — one probe per pixel, 1×1 octahedral map (just radiance along surface normal)
2. Upload chunk occupancy as a 3D texture
3. WGSL compute shader: for each screen pixel, reconstruct world position from depth, raymarch voxels in hemisphere, accumulate emissive hits
4. Apply as a simple diffuse irradiance term (multiply by albedo)
5. Debug visualization: show raw irradiance as a fullscreen overlay

**Deliverable:** Basic ambient lighting from emissive voxels. Validates the raymarch-through-voxels pipeline.

### Phase 3: Multi-Cascade Hierarchy

1. Implement N cascades (default 4) with the scaling law from Eq. 19:
   - Cascade *i*: probe spacing 2^i pixels, octahedral size 2^i × 2^i, interval [2^i, 2^(i+1)]
2. Pack all cascades into a single atlas texture
3. Implement back-to-front merge pass (Eq. 13)
4. Bilateral spatial interpolation (depth-aware) for cascade merging
5. Debug: visualize individual cascades, toggle cascade count

**Deliverable:** Full radiance cascade hierarchy with correct interval merging.

### Phase 4: Temporal Reprojection

1. Store previous frame's cascade atlas
2. Reproject each cascade using camera motion vectors
3. Blend reprojected data with fresh rays (configurable blend factor)
4. Invalidation: discard reprojected data where voxels changed (dirty chunks)

**Deliverable:** Amortized raymarching cost, smoother lighting under camera motion.

### Phase 5: Specular and Polish

1. Cone queries along reflection vector for specular indirect
2. Roughness-dependent cone angle (wide cone = diffuse, narrow = mirror)
3. Integration with ADR-0007 material properties (roughness, metalness)
4. Performance tuning: probe update budget per frame, LOD-aware cascade extent
5. UI controls: cascade count, interval range, debug overlays

**Deliverable:** Full diffuse + specular indirect lighting from radiance cascades.

---

## Performance Budget

Target: < 16ms total frame time (60 FPS). Radiance cascades budget: **4-8ms**.

| Pass | Estimated cost | Notes |
|------|---------------|-------|
| Depth prepass | 0.5-1ms | Shared with culling pipeline |
| Cascade build (4 cascades, with temporal) | 2-4ms | Amortized: ~25% of probes refreshed per frame |
| Cascade merge | 0.5-1ms | Simple per-texel compute |
| Lighting application | 0.5-1ms | Per-fragment hemisphere integration |
| **Total** | **3.5-7ms** | Within budget |

Without temporal reprojection (Phase 2-3), expect 15-30ms for the cascade build alone. Temporal is essential for real-time use.

---

## Memory Budget

At 1920×1080, RGBA16F (8 bytes per texel):

| Cascade | Probes | Texels/probe | Atlas pixels | Memory |
|---------|--------|-------------|-------------|--------|
| 0 | 1920×1080 | 1×1 | 1920×1080 | ~16 MB |
| 1 | 960×540 | 2×2 | 1920×1080 | ~16 MB |
| 2 | 480×270 | 4×4 | 1920×1080 | ~16 MB |
| 3 | 240×135 | 8×8 | 1920×1080 | ~16 MB |
| **Total** | | | | **~64 MB** |

Plus previous frame atlas for temporal: **~128 MB total**. Acceptable for a desktop WebGPU application.

---

## Consequences

### Positive

- **Dynamic GI** — fully reactive to voxel edits, camera movement, and lighting changes with no bake step
- **Emissive materials become useful** — ADR-0007's `emissive` field finally drives visible indirect light
- **Shared infrastructure** — depth prepass unblocks Hi-Z occlusion culling
- **Scene-independent cost** — testbed users can load arbitrarily complex models without GI cost changing
- **Graceful degradation** — fewer cascades or lower probe density = softer shadows, not noise

### Negative

- **WebGPU dependency** — compute shaders required; no WebGL2 fallback for radiance cascades (WebGL2 fallback renderer continues without GI)
- **Depth prepass overhead** — adds ~1ms even when GI is disabled (but shared with culling)
- **Temporal artifacts** — reprojection can ghost under fast camera motion; needs invalidation tuning
- **Complexity** — significant new compute pipeline alongside existing render path

### Constraints Introduced

- `threeBackend.ts` must expose an app-owned depth texture (Phase 1 prerequisite)
- Chunk occupancy data must be uploaded to GPU as a 3D texture each frame (or incrementally on dirty)
- Emissive material data must be available in a GPU-accessible format (extends ADR-0007 material data texture)
- WebGPU is required — GI feature is unavailable on WebGL2 fallback path

---

## Open Questions

1. **Octahedral vs. cubemap probe encoding.** The paper is encoding-agnostic. Octahedral maps are more cache-friendly and pack better into 2D atlases. Recommended but not yet validated in WGSL.

2. **Occupancy upload granularity.** Should the entire world's occupancy be in one 3D texture, or should cascades only raymarch loaded chunks? For large worlds, a clipmap-style windowed upload around the camera may be necessary.

3. **Interaction with LOD (ADR-0006).** Point-mode chunks at LOD 1 still have occupancy data — should distant cascades raymarch through them, or treat them as opaque at chunk granularity?

4. **Multi-bounce.** The paper focuses on single-bounce indirect. Multiple bounces can be achieved by feeding cascade output back as emissive input in subsequent frames. Worth exploring but not in initial scope.

---

## References

- Sannikov, A. "Radiance Cascades: A Novel Approach to Calculating Global Illumination." *Journal of Computer Graphics Techniques (JCGT)*, WIP. (`docs/RadianceCascades.pdf`)
- [ADR-0001](0001-renderer-choice.md) — Three.js with WebGPURenderer
- [ADR-0003](0003-binary-greedy-meshing.md) — Binary greedy meshing (provides opaque_mask)
- [ADR-0007](0007-material-strategy.md) — Material strategy (emissive properties)
- [ADR-0009](../../../docs/voxelizer-integration/adr/0009-architecture-b.md) — GPU-compact voxelizer integration
- [Hi-Z Occlusion Culling Report](../../culling/hiz-occlusion-culling-report.md) — Shared depth prepass prerequisite
- [ADR-0011](../../gpu-driven-rendering/adr/0011-hybrid-gpu-driven.md) — Hybrid GPU-driven pipeline (rendering infrastructure)
- [GPU-Driven Rendering Docs](../../gpu-driven-rendering/INDEX.md) — Full pipeline architecture and frame graph
