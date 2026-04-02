# Stage R-7: Cascade Merge

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Stage type:** GPU compute
**Trigger:** Every frame, after R-6 (cascade build). Back-to-front fold: cascade N-1 -> cascade N-2 -> ... -> cascade 0.

> Merges all cascade levels into a single unified radiance field in cascade 0. Each merge step folds a coarser cascade into a finer one using bilateral depth-weighted interpolation. Final output: merged cascade_atlas_0 ready for R-5/R-8 sampling.

---

## Purpose

After R-6 builds raw radiance at each cascade level independently, each level covers only its own distance interval. No single level has the full picture. R-7 folds all levels together back-to-front so that cascade 0 accumulates contributions from all distances. The merge uses the radiance interval equation (Sannikov Eq. 13) and bilateral interpolation weighted by depth similarity to prevent light leaking across depth discontinuities.

---

## Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-1 | `cascade_atlas[i]` for all levels `i` in `[0, N-1]` contains valid radiance and opacity from R-6 | R-6 postcondition |
| PRE-2 | All cascade atlas texels have `.rgb >= 0.0` and `.a` in `[0.0, 1.0]` | R-6 POST-1, POST-2 |
| PRE-3 | `depth_texture` contains valid depth from R-2 (unchanged since R-6 read it) | R-2 postcondition, DT-1 |
| PRE-4 | Inactive probes (far-plane depth) have all-zero texels in every cascade level | R-6 POST-3 |

---

## Inputs

| Buffer / Texture | Access | Format | What's read |
|---|---|---|---|
| `cascade_atlas[i+1]` | Read | `rgba16float` 2D | Higher (coarser) cascade level's radiance + opacity, to be folded into cascade `i` |
| `cascade_atlas[i]` | Read | `rgba16float` 2D | Current cascade level's own build result (radiance + opacity from R-6) |
| `depth_texture` | Read | `depth32float` | Per-pixel depth values for bilateral weight computation at probe positions and their neighbors |

---

## Transformation

### Merge Order

Back-to-front, one dispatch per cascade level pair:

```
For i = N-2 down to 0:
    dispatch cascade_merge.wgsl for cascade level i
    (reads cascade_atlas[i+1] merged result, reads cascade_atlas[i] build result, writes cascade_atlas[i] merged result)
```

The highest cascade (N-1) is never merged into -- it is the outermost interval and has no farther data to accumulate. The merge starts at N-2 and works down to 0.

### Per-Probe Merge

For each probe at grid position `(px, py)` in cascade `i`, and for each octahedral direction `(ox, oy)`:

#### 1. Read Own Build Result

```
own_radiance = cascade_atlas[i]  at (px * 2^i + ox, py * 2^i + oy)
L_i      = own_radiance.rgb
opacity_i = own_radiance.a
```

#### 2. Bilateral Interpolation of Coarser Cascade

Cascade `i+1` probes are spaced at `2^(i+1)` pixels. Cascade `i` probes are spaced at `2^i` pixels. Each cascade `i` probe position falls between four cascade `i+1` probes. Naive bilinear interpolation would bleed light across depth discontinuities (a probe on a wall receiving light as if it were behind the wall). Bilateral interpolation weights by depth similarity:

```
// The 4 nearest cascade i+1 probes surrounding this cascade i probe
for each neighbor (nx, ny) in the 2x2 footprint:
    neighbor_depth = textureSample(depth_texture, neighbor_screen_uv)
    probe_depth    = textureSample(depth_texture, probe_screen_uv)

    depth_diff = abs(probe_depth - neighbor_depth)
    sigma = 0.1 * probe_depth    // 10% of probe depth as threshold
    w = exp(-(depth_diff * depth_diff) / (2.0 * sigma * sigma))

    // Also compute spatial bilinear weight (distance within the 2x2 cell)
    bilinear_w = bilinear_weight(probe_subpixel_position, neighbor_index)

    total_weight += w * bilinear_w
    accumulated  += w * bilinear_w * cascade_atlas[i+1].sample(neighbor, direction)
```

Normalize: `L_coarser = accumulated / total_weight` (if `total_weight > 0`; otherwise `L_coarser = vec3f(0.0)`).

The direction mapping between cascade levels: cascade `i` has `2^i x 2^i` octahedral texels per probe, cascade `i+1` has `2^(i+1) x 2^(i+1)`. The merge samples the corresponding direction in the coarser cascade's octahedral map. Since the coarser cascade has higher angular resolution per probe, the direction lookup samples the texel(s) in the `2^(i+1) x 2^(i+1)` map that correspond to the same world-space direction as the current `(ox, oy)` texel in the finer cascade.

#### 3. Interval Merge Equation

Apply Sannikov Eq. 13:

```
L_merged(i) = L_i + (1 - opacity_i) * L_coarser
```

Where:
- `L_i` is this cascade level's own radiance from its interval `[t_i, t_{i+1}]`
- `opacity_i` is this cascade level's opacity (1 = ray hit opaque voxel in this interval)
- `L_coarser` is the bilateral-interpolated merged result from cascade `i+1` (covering `[t_{i+1}, ...]`)
- `(1 - opacity_i)` is the transmittance -- how much farther light reaches the probe

If this probe is opaque (`opacity_i = 1.0`), the coarser cascade contributes nothing. If this probe is transparent (`opacity_i = 0.0`), the coarser cascade contributes fully.

#### 4. Write Merged Result

```
merged_opacity = max(opacity_i, L_coarser_opacity)   // opacity from any interval blocks
cascade_atlas[i] at (px * 2^i + ox, py * 2^i + oy) = vec4f(L_merged, merged_opacity)
```

The write overwrites the build result in `cascade_atlas[i]` with the merged result. After the merge pass for level `i` completes, `cascade_atlas[i]` contains the accumulated radiance from interval `[t_i, infinity)`.

---

## Outputs

| Buffer / Texture | Access | Format | What's written |
|---|---|---|---|
| `cascade_atlas[i]` | Write (in-place) | `rgba16float` 2D | Merged radiance + opacity: `L_merged(i) = L_i + (1 - opacity_i) * L_(i+1)` per direction per probe |

After all merge passes complete, `cascade_atlas_0` contains the full merged radiance field covering all intervals `[0, t_N]`.

---

## Postconditions

| ID | Condition | Validates |
|---|---|---|
| POST-1 | `cascade_atlas_0` contains merged radiance from all N cascade levels | CAS-7 (full merged result) |
| POST-2 | All merged texels have `.rgb >= 0.0` | CAS-4 (non-negative radiance preserved through merge) |
| POST-3 | All merged texels have `.a` in `[0.0, 1.0]` | CAS-5 (opacity bounds preserved) |
| POST-4 | Inactive probes (far-plane depth) remain all-zero after merge | CAS-6 (inactive probes unchanged) |
| POST-5 | For each cascade level `i`, merged radiance is >= the level's own build radiance (merge only adds, never subtracts) | Monotonic accumulation |
| POST-6 | No light bleeds across depth discontinuities exceeding the bilateral sigma threshold | Bilateral weight correctness |

---

## Dispatch

```
Per cascade level pair (i = N-2 down to 0):
  workgroup_size: (8, 8, 1)
  dispatch_x: ceil((screen_w / 2^i) / 8)
  dispatch_y: ceil((screen_h / 2^i) / 8)
  dispatch_z: 1
```

Total dispatches per frame: `N_CASCADES - 1` (typically 3 for 4 cascades).

Each workgroup handles 64 probes. Each thread handles one probe and iterates over its `2^i x 2^i` octahedral directions sequentially, performing the bilateral sample and merge equation for each direction.

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Bilateral weight function:** Verify `bilateral_weight(d, d) == 1.0` (identical depths). Verify weight approaches 0 as depth difference exceeds sigma. Verify sigma scales with probe depth.
2. **Merge equation identity:** With `opacity_i = 0.0` and `L_i = 0.0`, verify `L_merged == L_coarser`. With `opacity_i = 1.0`, verify `L_merged == L_i` regardless of `L_coarser`.
3. **Weight normalization:** Verify the 4-neighbor bilateral weights sum to a positive value for non-degenerate cases, and the result is properly normalized.

### GPU validation

4. **Single cascade (no merge):** With N=1, verify R-7 is a no-op and `cascade_atlas_0` retains its R-6 build result unchanged.
5. **Two cascades, uniform depth:** Build cascade 0 with zero radiance (empty near-field) and cascade 1 with known radiance. After merge, verify cascade 0 contains cascade 1's radiance (fully transparent near-field passes through).
6. **Depth discontinuity:** Place two probes at very different depths adjacent in screen space. Verify bilateral interpolation suppresses cross-leak (probe on foreground surface does not receive background probe's radiance from the coarser cascade).
7. **Monotonic accumulation:** For random scenes, verify merged cascade 0 radiance is >= cascade 0 build radiance at every texel.

### Cross-stage tests

8. **R-6 -> R-7:** Verify merge pass produces no NaN or inf values when consuming R-6 output.
9. **R-7 -> R-5:** Verify `cascade_atlas_0` is readable by the color pass fragment shader and contains valid radiance.
10. **R-7 -> R-8:** Verify the merged cascade 0 produces visually correct GI contribution when composited with the color target.

---

## See Also

- [radiance-cascades-impl](../radiance-cascades-impl.md) -- merge algorithm, bilateral interpolation, interval merge equation (Sannikov Eq. 13)
- [pipeline-stages](../pipeline-stages.md) -- R-7 buffer ownership and stage ordering
- [cascade-atlas](../data/cascade-atlas.md) -- atlas layout, memory budget, constant-size property
- [depth-texture](../data/depth-texture.md) -- depth buffer consumed for bilateral weights
- [R-6-cascade-build](R-6-cascade-build.md) -- upstream stage that produces per-level raw radiance
