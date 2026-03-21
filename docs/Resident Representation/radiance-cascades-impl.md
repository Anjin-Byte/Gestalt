# Radiance Cascades Implementation

Cascade build, merge, and apply passes grounded in the GPU-resident chunk runtime.

This document is the implementation spec. For the architectural decision and rationale see [[../greedy-meshing-docs/adr/0010-radiance-cascades]].

---

## Recap: The Chosen Variant

**Hybrid — screenspace probe placement, world-space voxel raymarching.**

Probes sit on depth-buffer surface positions (screen-proportional memory). Rays march through `chunk_occupancy_atlas` (world-space truth). This variant:

- Costs memory proportional to screen area, not scene volume
- Captures off-screen emissive geometry that pure screenspace variants miss
- Consumes the same chunk occupancy data the rest of the engine already maintains
- Requires the `depth_texture` produced by Stage R-2 (the depth prepass)

No architectural decisions are required from this document. They are already made.

---

## Layer Model Position

Radiance cascades span two layers:

| Component | Layer | Description |
|---|---|---|
| Cascade build rays (R-6) | Product 1 | World-space segment stream queries over chunk occupancy |
| Cascade merge (R-7) | Derived intermediate | Bilateral blend from high → low cascade |
| GI application (R-5 / R-8) | Product 2 consumer | Fragment-level hemisphere integration from merged cascade 0 |

Cascade build **must not** be filtered by Hi-Z, frustum cull results, or any Product 3 data. A probe behind the camera may march rays through chunks occluded from the camera. Those chunks are still valid emissive sources. See [[layer-model]].

---

## Cascade Atlas Layout

Each cascade level is a 2D texture atlas. Each cascade has the same atlas pixel area regardless of level — probe count halves per spatial dimension but per-probe directional resolution doubles per dimension, keeping total texels constant per level. Total memory therefore scales **linearly** with cascade count (Sannikov Section 2.5.3, surface radiance cascades).

```
Cascade i:
  probe_grid:      (screen_w / 2^i) × (screen_h / 2^i)    probes
  per_probe_map:   (2^i × 2^i) texels (octahedral encoding)
  atlas_size:      screen_w × screen_h texels  (constant per cascade)
  texel_format:    rgba16float  (radiance RGB + opacity alpha)
  interval:        [t_i, t_{i+1}]  where t_0 = 0, t_i ~ 2^(i-1) for i ≥ 1
```

The interval boundaries are set by `t_0 = 0` (Sannikov Eq. 18). A practical parameterization for a voxel world with 1-voxel minimum feature size:

| Cascade | Probe grid | Per-probe | Interval (voxels) | Atlas size |
|---|---|---|---|---|
| 0 | 1920×1080 | 1×1 | [0, 1] | 1920×1080 |
| 1 | 960×540 | 2×2 | [1, 2] | 1920×1080 |
| 2 | 480×270 | 4×4 | [2, 4] | 1920×1080 |
| 3 | 240×135 | 8×8 | [4, 8] | 1920×1080 |

Cascade 0 must start at 0 to capture near-field contact shadows — the most spatially sharp part of the lighting. Starting cascade 0 at 1 would leave the entire near-field range unsampled.

Total GPU memory: 4 cascades × 1920×1080 × 8 bytes (rgba16f) ≈ **64 MB**.
Plus one previous-frame set if temporal reprojection is enabled: **~128 MB total**.

---

## GPU Buffer Set

### Cascade Atlases

```
cascade_atlas[N_CASCADES]          rgba16float 2D texture, per cascade level
cascade_atlas_prev[N_CASCADES]     rgba16float 2D texture, previous frame (temporal)
```

### Per-Pass Uniforms

```
cascade_uniforms:
  screen_size:      vec2u       (width, height)
  proj_inv:         mat4f       (inverse projection, for depth → view-space unproject)
  view_inv:         mat4f       (inverse view, for view-space → world-space)
  cascade_index:    u32         (which cascade level this dispatch builds)
  frame_index:      u32         (monotonic, for temporal blend and noise offset)
  temporal_alpha:   f32         (blend weight for previous frame data, e.g. 0.1)
  voxel_scale:      f32         (world units per voxel — unifies interval math)
```

### Emissive Lookup

```
emissive_lookup:   array<vec4f>  per material ID — emissive RGB + intensity
                                  populated from chunk_palette_buf on CPU side
                                  updated when materials change
```

No new chunk pool buffers are required. The cascade system reads from:
- `depth_texture` — probe position reconstruction
- `chunk_occupancy_atlas` — occupancy test during raymarching
- `occupancy_summary_buf` — bricklet skip during raymarching
- `chunk_flags_buf` — `is_empty` and `has_emissive` chunk skip
- `chunk_slot_table_gpu` — world coordinate → slot index lookup
- `chunk_palette_buf` — emissive radiance at confirmed hit

All of these already exist in the chunk pool (see [[gpu-chunk-pool]]). The cascade system is a new consumer of existing data — a clean Layer 3 addition.

---

## Probe Placement

Each probe is anchored to a surface point derived from the depth buffer.

```wgsl
// cascade_build.wgsl — probe world position
fn probe_world_pos(screen_uv: vec2f, depth: f32) -> vec3f {
    // NDC
    let ndc = vec3f(screen_uv * 2.0 - 1.0, depth);
    // View space
    let view_h = cascade_uniforms.proj_inv * vec4f(ndc, 1.0);
    let view_pos = view_h.xyz / view_h.w;
    // World space
    let world_h = cascade_uniforms.view_inv * vec4f(view_pos, 1.0);
    return world_h.xyz;
}
```

For cascade level `i`, probes are spaced `2^i` pixels apart. Each probe at grid position `(px, py)` samples the depth buffer at `(px * 2^i + 2^(i-1), py * 2^i + 2^(i-1))` — the center of its footprint.

If the depth sample returns the far-plane value (no surface), the probe is inactive. Its atlas texels are written as `vec4f(0.0)`.

---

## Stage R-6: Cascade Build

One compute dispatch per cascade level, from highest to lowest.

```
Dispatch: one workgroup per probe tile
  workgroup size: (8, 8, 1) → 64 probes per workgroup

Per probe:
  1. Sample depth_texture at probe screen position
  2. If no surface: write zero radiance, skip
  3. Reconstruct probe world position (depth + inv_proj + inv_view)
  4. For each direction in this probe's octahedral map (2^i × 2^i texels):
     a. Decode octahedral direction to world-space vec3
     b. Call traceSegments(probe_world_pos, direction, t_start=t_i, t_end=t_{i+1})
        where t_0=0, t_1=1, t_2=2, t_3=4, t_4=8 (per table above)
     c. Accumulate emissive radiance from OpaqueSegment hits
     d. Track transparency: if ray blocked, alpha = 0; if clear, alpha = 1
  5. Blend with temporal: mix(fresh_radiance, reprojected_prev_radiance, alpha=temporal_alpha)
  6. Write rgba16float to cascade_atlas[cascade_index] at probe's octahedral texel range
```

### Ray Interval Convention

Cascade 0 traces interval `[0, 1]` — starting from the probe surface itself. This is non-negotiable: `t_0 = 0` (Sannikov Eq. 18). Starting cascade 0 from any positive value leaves a near-field gap that no other cascade fills.

Cascade i traces `[t_i, t_{i+1}]` where `t_i ~ 2^(i-1)` for i ≥ 1 (consistent with the table above).

After merging, cascade 0 accumulates contributions from all distances as higher cascades are folded in from back to front. No single cascade sees the full ray — each is responsible for its slice.

The short intervals in lower cascades are what makes per-probe cost tractable. Cascade 0 traces only 1 voxel unit per probe. Cascade 3 traces 4 voxel units but has 64× fewer probes (8× fewer in each of two dimensions).

---

## The Traversal Call

Each probe direction calls `traceSegments` as defined in [[traversal-acceleration]].

```wgsl
// Inside cascade_build.wgsl, per probe direction
// t_0 = 0 (paper Eq. 18); t_i ~ 2^(i-1) for i >= 1
let t_start = select(0.0, f32(1u << (cascade_index - 1u)), cascade_index > 0u);
let t_end   = select(1.0, f32(1u << cascade_index),        cascade_index > 0u);

let interval_result = traceSegments(
    probe_pos,
    direction,
    t_start,
    t_end,
);

var radiance = vec3f(0.0);
var opacity  = 0.0;

for segment in interval_result {
    switch segment {
        case OpaqueSegment(t_enter, t_exit, voxel) {
            // Look up material, fetch emissive value
            let slot = world_coord_to_slot(voxel >> 6);   // chunk coord
            let local = voxel & 63;
            let mat_id = fetch_material_id(slot, local);
            let emissive = emissive_lookup[mat_id].rgb * emissive_lookup[mat_id].a;
            radiance += emissive;
            opacity = 1.0;
            break;  // Opaque: no further contribution in this interval
        }
        case EmptySegment(t_enter, t_exit) {
            // No contribution — traversal skipped empty region
            // traceSegments may emit empty segments as a by-product of chunk/bricklet skip
        }
    }
}
```

**Chunk skip path:** Before `traceSegments` descends into a chunk, it checks `chunk_flags_buf[slot].is_empty`. For GI purposes, a second optimization is available: if `!chunk_flags_buf[slot].has_emissive`, the chunk contains no light sources. For first-bounce cascade build, emissive-free chunks still contribute opacity (they block light). Do not skip non-emissive chunks from opacity tracking.

**Slot lookup:** `world_coord_to_slot(chunk_coord)` reads `chunk_slot_table_gpu`. This is the same flat lookup used by all other GPU traversal consumers.

---

## Stage R-7: Cascade Merge

After all cascade levels are built, merge back-to-front: cascade `N-1` into `N-2`, down to cascade 0.

```
For i = N-2 down to 0:
  dispatch cascade_merge.wgsl for cascade level i
```

Each merge pass folds cascade `i+1` into cascade `i` using the radiance interval equation (Sannikov Eq. 13):

```
L_merged(i) = L_i + (1 - opacity_i) × L_(i+1)
```

Where:
- `L_i` is the radiance at cascade `i`'s interval `[2^i, 2^(i+1)]`
- `L_(i+1)` is the merged result from cascade `i+1` (covering `[2^(i+1), ∞)`)
- `(1 - opacity_i)` is the transmittance from cascade `i` — how much of the farther light reaches the probe

This is evaluated per direction (per octahedral texel) within each probe.

### Bilateral Spatial Interpolation

When cascade `i+1` probes are at `2^(i+1)` pixel spacing and cascade `i` probes are at `2^i` pixel spacing, cascade `i+1` data must be interpolated to align with each cascade `i` probe.

Naive bilinear interpolation bleeds light across depth discontinuities — a probe on a wall receives light as if it were behind the wall. Bilateral interpolation weights by depth similarity:

```wgsl
fn bilateral_weight(probe_depth: f32, neighbor_depth: f32) -> f32 {
    let depth_diff = abs(probe_depth - neighbor_depth);
    let sigma = 0.1 * probe_depth;  // 10% of probe depth as threshold
    return exp(-(depth_diff * depth_diff) / (2.0 * sigma * sigma));
}
```

The merge pass fetches the 4 nearest cascade `i+1` probes, weights by bilateral depth similarity, and normalizes.

The `depth_texture` from Stage R-2 provides the per-pixel depth values for both the probe positions and their neighbors.

---

## Stage R-5 / R-8: GI Application

After merge completes, cascade 0 holds the full merged radiance field for the scene.

Applied in the main color pass fragment shader (inline R-5 integration):

```wgsl
// color.wgsl fragment shader — GI term
fn sample_cascade_0(world_pos: vec3f, normal: vec3f) -> vec3f {
    // Convert world_pos to screen UV of nearest cascade-0 probe
    let screen_uv = world_to_screen_uv(world_pos);

    // Sample cascade 0 at this screen position
    // Each texel in cascade 0 covers one direction (1×1 octahedral = single radiance value)
    // For cascade 0 at 1×1 per probe, the probe itself IS the texel
    let probe_radiance = textureSample(cascade_atlas[0], sampler, screen_uv);

    // Diffuse: integrate hemisphere of probe directions weighted by cos(theta)
    // For cascade 0 (1×1 per probe), apply as ambient irradiance
    let diffuse_gi = probe_radiance.rgb * material_albedo * INV_PI;

    return diffuse_gi;
}
```

For higher cascades (2×2, 4×4 per probe), sample the octahedral map in the hemisphere aligned with `normal` for more directional GI fidelity.

The direct lighting term (emissive self-emission + any explicit light sources) is added separately.

---

## Temporal Reprojection

**The base behavior is fresh rays every frame.** The paper (p. 3) states that radiance cascades build "from scratch every frame without reusing any data from the previous frame" — that is a core stated property and a correctness advantage: no ghosting, no temporal lag when voxels change.

Temporal reprojection is an optional optimization (Section 4.5: "This implementation *also* utilizes reprojection"). It trades convergence latency for per-frame cost. At the expected 30–50ms per frame for the hybrid world-space variant without reprojection, temporal amortization is practical but not required for correctness.

When disabled: all probes are rebuilt fresh each frame. Correct but expensive.
When enabled: only a fraction of probes receive fresh rays per frame — convergence time is traded for per-frame cost.

### Reprojection Protocol

```
Per frame:
  1. Reproject cascade_atlas_prev[i] using camera motion delta
     For each probe in cascade i:
       prev_screen_uv = current_screen_uv - motion_vector(screen_uv)
       reprojected = sample cascade_atlas_prev[i] at prev_screen_uv
  2. Blend fresh rays with reprojected:
       final = mix(fresh_radiance, reprojected, temporal_alpha)
       where temporal_alpha = 0.9 for slow convergence / 0.1 for fast refresh
  3. Copy cascade_atlas[i] → cascade_atlas_prev[i] at end of frame
```

### Invalidation on Voxel Edit

When chunks are dirtied (edit protocol dirty bits — see [[edit-protocol]]), probes whose rays intersect those chunks may hold stale radiance. Invalidation strategy:

```
For each dirty chunk (from dirty_chunks bitset):
  Project chunk AABB onto screen
  For each cascade level i:
    For each probe in the screen-space projection:
      Set temporal_alpha = 0.0 for that probe (force fresh ray this frame)
```

This is conservative (over-invalidates slightly) but cheap. Probe invalidation is a compute dispatch reading `dirty_chunks` — already a GPU-resident buffer from the edit protocol.

At Stage 3 GPU-driven scheduling, this pass runs automatically as part of the propagation pass output.

---

## Integration with the Demo Renderer

The demo renderer (see [[demo-renderer]]) targets cascade 0 only for the MVP.

**Cascade 0 simplifications:**
- 1×1 per-probe octahedral map = no directional interpolation needed — each probe is one radiance value (scalar irradiance along the surface normal)
- No merge pass needed (no cascades to fold in)
- Temporal blend is optional for the MVP — full refresh every frame
- GI application: multiply `probe_radiance.rgb × material_albedo` as ambient irradiance

This gives a cheap, directionally-limited but visually useful ambient occlusion + emissive lighting term with minimal shader complexity.

**Activation sequence in the demo:**

```
Demo init:
  Allocate cascade_atlas[0] (rgba16float, screen_w × screen_h)
  Allocate cascade_atlas_prev[0] (same)
  Populate emissive_lookup from chunk_palette_buf

Per frame:
  After R-2 (depth prepass):
    Dispatch cascade_build.wgsl with cascade_index=0
  After cascade build:
    // No merge pass for single cascade
  In R-5 color pass:
    Bind cascade_atlas[0] as read texture
    Apply as ambient GI term in fragment shader
```

---

## Shader File Summary

```
passes/cascadeBuild.ts      — R-6 dispatch: one call per cascade level, high to low
passes/cascadeMerge.ts      — R-7 dispatch: one call per cascade pair, back to front
shaders/cascade_build.wgsl  — compute: probe placement, traceSegments, temporal blend
shaders/cascade_merge.wgsl  — compute: bilateral interpolation, interval merge equation
shaders/cascade_common.wgsl — shared: octahedral encode/decode, bilateral weight, slot lookup
```

`cascade_common.wgsl` is imported by both build and merge shaders. It contains:
- `octahedral_encode(dir: vec3f) → vec2f`
- `octahedral_decode(uv: vec2f) → vec3f`
- `probe_world_pos(screen_uv, depth) → vec3f`
- `world_coord_to_slot(chunk_coord: vec3i) → u32`
- `bilateral_weight(depth_a, depth_b) → f32`

---

## Memory Budget

At 1920×1080, 4 cascades, RGBA16F (8 bytes/texel):

| Buffer | Size | Notes |
|---|---|---|
| `cascade_atlas[0..3]` | 4 × 16 MB = 64 MB | Current frame |
| `cascade_atlas_prev[0..3]` | 64 MB | Previous frame, temporal |
| `emissive_lookup` | N_materials × 16 bytes ≈ negligible | Per-material emissive |
| `cascade_uniforms` | < 1 KB | Per-pass |
| **Total** | **~128 MB** | |

Cascade memory scales with screen resolution, not scene size. Halving resolution halves cascade memory. This is independent of the chunk pool budget (see [[gpu-chunk-pool]]).

---

## What Still Needs Design

| Component | Status | Notes |
|---|---|---|
| `traceSegments` WGSL implementation | Needs impl | Depends on chunk DDA and voxel DDA from [[traversal-acceleration]] |
| Octahedral map sampling for N>1 cascade | Needs design | Hemisphere integration over 2^i × 2^i texels per probe |
| Specular cone query | Deferred | Roughness → cone angle → directional cascade sample |
| Multi-bounce | Deferred | Feed cascade output as emissive source in subsequent frame |
| LOD interaction | Open question | Should cascade rays use point-mode chunks at far distances? |
| Volumetric probes (far field) | Deferred | Probes in empty space, not on surfaces |
| Probe update budget limiting | Deferred | Per-frame budget cap on refreshed probes (Stage 3 scheduling) |

---

## See Also

- [[traversal-acceleration]] — `traceSegments` contract; `traceSegments` is the cascade build ray kernel
- [[pipeline-stages]] — Stage R-6 (cascade build), R-7 (merge), R-5 (GI application); buffer read/write ownership
- [[gpu-chunk-pool]] — `chunk_slot_table_gpu`, `chunk_flags_buf`, `occupancy_summary_buf`; all consumed read-only
- [[edit-protocol]] — dirty chunk tracking; cascade probe invalidation reads `dirty_chunks`
- [[layer-model]] — why cascade build is Product 1 (world-space, not camera-filtered)
- [[demo-renderer]] — cascade 0 MVP integration
- [[../greedy-meshing-docs/adr/0010-radiance-cascades]] — ADR: decision rationale, variant comparison, memory math
