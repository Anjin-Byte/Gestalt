# Cascade Atlas

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Per-frame transient (with temporal history).

> The radiance cascade atlases. One 2D texture per cascade level, storing probe radiance in octahedral encoding. The intermediate representation between world-space ray traversal and screen-space GI application.

---

## Identity

- **Texture names:** `cascade_atlas_0` through `cascade_atlas_N` (one per cascade level); `cascade_atlas_prev_0` through `cascade_atlas_prev_N` (previous frame, temporal)
- **Format:** `rgba16float` (radiance RGB + opacity alpha)
- **GPU usage:** `TEXTURE_BINDING | STORAGE_BINDING` (read in merge/apply, written by build/merge)
- **Binding:** `@group(1) @binding(0..N)` in cascade build and merge shaders; `@group(2) @binding(0)` for cascade 0 in the color pass (R-5)

---

## Layout

Each cascade level is a single 2D texture atlas with **constant pixel dimensions** regardless of cascade level. Probe count halves per spatial dimension while per-probe directional resolution doubles per dimension, keeping total texels constant.

```
Cascade i:
  probe_grid:    (screen_w / 2^i) x (screen_h / 2^i)   probes
  per_probe_map: (2^i x 2^i) texels                     octahedral encoding
  atlas_size:    screen_w x screen_h texels              constant per cascade
  texel_format:  rgba16float                             8 bytes per texel
```

For a probe at grid position `(px, py)` in cascade `i`, its octahedral texels occupy:

```
atlas_x = px * 2^i + ox       where ox in [0, 2^i - 1]
atlas_y = py * 2^i + oy       where oy in [0, 2^i - 1]
```

Each octahedral texel `(ox, oy)` encodes a direction via octahedral mapping. The `.rgb` channels store accumulated radiance along that direction; the `.a` channel stores opacity (0 = ray was unblocked through the cascade interval, 1 = ray hit an opaque voxel).

### Reference Table (1920x1080)

| Cascade | Probe grid | Per-probe | Ray interval (voxels) | Atlas size |
|---|---|---|---|---|
| 0 | 1920x1080 | 1x1 | [0, 1] | 1920x1080 |
| 1 | 960x540 | 2x2 | [1, 2] | 1920x1080 |
| 2 | 480x270 | 4x4 | [2, 4] | 1920x1080 |
| 3 | 240x135 | 8x8 | [4, 8] | 1920x1080 |

### Why constant atlas size?

This is the core memory scaling property of radiance cascades (Sannikov Section 2.5.3). Each higher cascade has fewer probes but more directions per probe. The product is constant. Total memory scales linearly with cascade count, not exponentially.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| CAS-1 | Each cascade atlas has exactly `screen_w x screen_h` texels | Texture allocation on resize |
| CAS-2 | All cascade atlases share the same `rgba16float` format | Pipeline creation |
| CAS-3 | Cascade `i` contains `(screen_w / 2^i) x (screen_h / 2^i)` probes, each with `2^i x 2^i` octahedral texels | Layout definition |
| CAS-4 | `.rgb` values are non-negative (radiance is non-negative by definition) | Cascade build shader clamp |
| CAS-5 | `.a` is in `[0.0, 1.0]` (opacity) | Cascade build shader clamp |
| CAS-6 | Probes at far-plane depth positions (no surface) have all-zero texels | Cascade build: inactive probe write |
| CAS-7 | After merge (R-7), `cascade_atlas_0` contains the full merged radiance from all cascade levels | Merge pass correctness |
| CAS-8 | `cascade_atlas_prev_i` contains the previous frame's data for cascade `i` at the time R-6 begins | End-of-frame copy |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| `.rgb` (radiance) | `[0.0, 65504.0]` | fp16 max; practically bounded by emissive intensity in `material_table` |
| `.a` (opacity) | `[0.0, 1.0]` | 0 = fully transparent interval, 1 = fully blocked |
| Probe grid position | `(px, py)` where `px in [0, screen_w/2^i)`, `py in [0, screen_h/2^i)` | Per-cascade bounds |
| Octahedral texel | `(ox, oy)` where `ox, oy in [0, 2^i)` | Per-cascade directional resolution |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Cascade build (R-6) | Every frame, one dispatch per cascade level | Per-probe radiance + opacity from world-space ray traversal through `chunk_occupancy_atlas` |
| Cascade merge (R-7) | Every frame, after R-6, back-to-front | Merged radiance: `L_merged(i) = L_i + (1 - opacity_i) * L_(i+1)` per direction per probe |
| Temporal copy | End of frame | `cascade_atlas[i]` copied to `cascade_atlas_prev[i]` for next frame's temporal blend |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Cascade merge | R-7 | Reads cascade `i+1` to merge into cascade `i` via bilateral interpolation |
| GI application | R-5 (inline fragment) | Reads merged `cascade_atlas_0` to integrate hemisphere irradiance for diffuse GI |
| Debug visualization | R-9 | Reads any cascade atlas for radiance heatmap overlay |

---

## Memory Budget

At 1920x1080, `rgba16float` (8 bytes per texel):

| Resource | Size | Notes |
|---|---|---|
| One cascade atlas | 1920 x 1080 x 8 = ~16 MB | Constant per level |
| 4 cascade levels (current frame) | ~64 MB | `cascade_atlas_0` through `cascade_atlas_3` |
| 4 cascade levels (previous frame) | ~64 MB | `cascade_atlas_prev_0` through `cascade_atlas_prev_3` (temporal) |
| **Total** | **~128 MB** | With temporal reprojection enabled |
| **Total (no temporal)** | **~64 MB** | Without temporal reprojection |

Memory scales with screen resolution, not scene size. Halving resolution quarters cascade memory. This budget is independent of the chunk pool budget (see [gpu-chunk-pool](../gpu-chunk-pool.md)).

---

## UNDERSPECIFIED: Single Texture vs. Texture Array

The narrative docs leave unresolved whether cascade atlases should be:

**Option A -- Separate 2D textures (one per cascade level)**
- `cascade_atlas_0: texture_2d<f32>`, `cascade_atlas_1: texture_2d<f32>`, ...
- Simpler binding model: each shader dispatch binds exactly the textures it needs
- More binding slots consumed (one per cascade level per pass)
- Current assumption in [pipeline-stages](../pipeline-stages.md) and [radiance-cascades-impl](../radiance-cascades-impl.md)

**Option B -- 2D texture array (all cascade levels in one resource)**
- `cascade_atlas: texture_2d_array<f32>` with `array_layer = cascade_index`
- Single binding for all levels; cascade index is a shader parameter
- Requires all levels to have identical dimensions (satisfied by the constant-atlas-size property)
- More efficient for merge pass (reads two adjacent layers in one binding)

**Decision needed before implementation.** Both options have identical memory cost. The choice affects shader binding layout and dispatch ergonomics but not correctness or memory.

---

## Temporal Reprojection

When temporal reprojection is enabled, the system maintains a double buffer:

- `cascade_atlas[i]` -- current frame, written by R-6 and R-7
- `cascade_atlas_prev[i]` -- previous frame, read by R-6 for temporal blend

At the end of each frame, current is copied to prev:
```
for i in 0..N_CASCADES:
  copyTextureToTexture(cascade_atlas[i], cascade_atlas_prev[i])
```

This doubles the memory cost (~128 MB total at 1080p with 4 cascades).

Temporal blend weight is controlled by `cascade_uniforms.temporal_alpha`:
- `temporal_alpha = 0.0` -- fully fresh (no reprojection, equivalent to disabled)
- `temporal_alpha = 0.9` -- heavy reuse (slow convergence, low per-frame cost)

Voxel edits force `temporal_alpha = 0.0` for affected probes via the edit protocol's `dirty_chunks` bitset projected onto screen space (see [radiance-cascades-impl](../radiance-cascades-impl.md), Invalidation on Voxel Edit).

---

## Testing Strategy

### Unit tests (Rust / TypeScript, CPU-side)

1. **Allocation dimensions:** For screen sizes 1920x1080, 1280x720, and 3840x2160, verify each cascade atlas is exactly `screen_w x screen_h` texels.
2. **Probe grid math:** For each cascade level, verify `probe_grid * per_probe_map == atlas_size`.
3. **Texel addressing:** For probe (px, py) in cascade i, verify atlas coordinates `(px * 2^i + ox, py * 2^i + oy)` are within atlas bounds for all valid `(ox, oy)`.
4. **Zero initialization:** Verify newly allocated cascade atlases are zeroed (no stale radiance from previous scene).

### Property tests (randomized)

5. **Non-negative radiance:** After R-6 dispatch with random emissive materials, readback cascade atlas and verify all `.rgb >= 0.0`.
6. **Opacity bounds:** Verify all `.a` values are in `[0.0, 1.0]`.
7. **Inactive probe zeroing:** For probes at far-plane depth, verify all four channels are zero.
8. **Constant atlas size:** For 10 random screen resolutions, verify all cascade levels produce the same atlas pixel count.

### GPU validation

9. **Merge correctness:** Build cascades with a single emissive voxel at known distance. Verify merged cascade 0 contains the expected radiance at the probes whose rays reach it.
10. **Temporal copy fidelity:** Write known pattern to `cascade_atlas[0]`, execute end-of-frame copy, readback `cascade_atlas_prev[0]`, verify byte-for-byte match.

### Cross-stage tests

11. **R-6 -> R-7:** After cascade build, verify merge pass produces monotonically non-decreasing radiance in cascade 0 (more cascades merged = more accumulated light).
12. **R-7 -> R-5:** Verify color pass fragment shader reads valid radiance from `cascade_atlas_0` (no NaN, no negative).
13. **Edit invalidation:** Dirty a chunk, verify affected probes receive `temporal_alpha = 0.0` and produce fresh rays rather than reprojected stale data.

---

## See Also

- [radiance-cascades-impl](../radiance-cascades-impl.md) -- cascade build, merge, and apply passes; traversal call; temporal reprojection protocol
- [pipeline-stages](../pipeline-stages.md) -- R-6 (cascade build), R-7 (cascade merge), R-5 (GI application) buffer ownership
- [gpu-chunk-pool](../gpu-chunk-pool.md) -- chunk pool buffers consumed by cascade build (occupancy, flags, palette, slot table)
- [edit-protocol](../edit-protocol.md) -- dirty chunk tracking for cascade probe invalidation
- [chunk-occupancy-atlas](chunk-occupancy-atlas.md) -- occupancy data read by cascade build rays during DDA traversal
