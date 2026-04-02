# Stage R-6: Radiance Cascade Build

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Stage type:** GPU compute
**Trigger:** Every frame, after R-5 (main color pass). One dispatch per cascade level (typically 4 levels, highest to lowest).

> Places probes on depth-buffer surface positions (screenspace). Each probe traces rays through world-space chunk_occupancy_atlas via DDA traversal (traceSegments). Writes cascade_atlas at the current level.

---

## Purpose

Build the raw radiance data for each cascade level. Each probe sits on a depth-buffer surface point and marches rays through the world-space chunk occupancy structure to detect emissive voxels and track opacity. The output — per-probe, per-direction radiance and opacity — feeds the merge pass (R-7) which folds all cascade levels into a single merged radiance field.

Each cascade level covers a specific distance interval `[t_i, t_{i+1}]` from the probe surface. Cascade 0 starts at `t=0` for contact shadows — the most spatially sharp part of the lighting. No other cascade fills the near-field range. The short intervals in lower cascades are what makes per-probe cost tractable: cascade 0 traces only 1 voxel unit per probe, while cascade 3 traces 4 voxel units but has 64x fewer probes.

---

## Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-1 | `depth_texture` contains valid depth from all chunk geometry | R-2 postcondition |
| PRE-2 | `chunk_occupancy_atlas` contains valid occupancy for all resident chunks | I-2 postcondition |
| PRE-3 | `chunk_flags` contains valid `is_empty` and `has_emissive` bits | I-3 postcondition |
| PRE-4 | `occupancy_summary` contains valid bricklet occupancy bits | I-3 postcondition |
| PRE-5 | `chunk_slot_table_gpu` maps world chunk coordinates to slot indices | Pool manager |
| PRE-6 | `material_table` is populated with valid material entries | Scene init |
| PRE-7 | `cascade_uniforms` contains current frame's inverse projection, inverse view, screen size, cascade index, and voxel scale | Per-dispatch uniform write |
| PRE-8 | `cascade_atlas_prev[i]` contains previous frame's cascade data (if temporal reprojection enabled) | End-of-previous-frame copy |

---

## Inputs

| Buffer / Texture | Access | Format | What's read |
|---|---|---|---|
| `depth_texture` | Read | `depth32float` | Probe screen position -> depth sample for world-position reconstruction |
| `camera_uniform` | Read | `mat4x2` (view + projection) | Inverse projection + inverse view for depth unproject |
| `cascade_uniforms` | Read | Struct | `screen_size`, `proj_inv`, `view_inv`, `cascade_index`, `frame_index`, `temporal_alpha`, `voxel_scale` |
| `chunk_occupancy_atlas` | Read | `array<u32>` (8192 per slot) | Per-column occupancy bits during DDA voxel march |
| `chunk_flags` | Read | `array<u32>` | `is_empty` for chunk skip; `has_emissive` for emissive-only optimization |
| `occupancy_summary` | Read | `array<u32>` (16 per slot) | 512-bit bricklet grid for empty bricklet skip |
| `chunk_slot_table_gpu` | Read | `array<ChunkSlotEntry>` | World coordinate -> slot index lookup during traversal |
| `chunk_palette_buf` | Read | Per-slot | Palette index -> MaterialId lookup after hit confirmed |
| `palette_meta` | Read | `array<u32>` per slot | `palette_size` + `bits_per_entry` for index unpack |
| `material_table` | Read | `array<MaterialEntry>` | Emissive RGB + intensity after palette resolution |
| `cascade_atlas_prev[i]` | Read | `rgba16float` 2D | Previous frame data for temporal blend (optional) |

---

## Transformation

### Dispatch Structure

One compute dispatch per cascade level, from highest (N-1) to lowest (0).

```
workgroup_size: (8, 8, 1)  -- 64 probes per workgroup
dispatch: ceil(probe_grid_w / 8) x ceil(probe_grid_h / 8) x 1

Where probe_grid for cascade i:
  probe_grid_w = screen_w / 2^i
  probe_grid_h = screen_h / 2^i
```

### Per-Probe Algorithm

For each probe at grid position `(px, py)` in cascade `i`:

#### 1. Probe Placement

Sample the depth buffer at the center of the probe's screen footprint:

```
screen_pixel = (px * 2^i + 2^(i-1), py * 2^i + 2^(i-1))
screen_uv = screen_pixel / screen_size
depth = textureSample(depth_texture, screen_uv)
```

If depth is the far-plane value (no surface), the probe is inactive. Write `vec4f(0.0)` to all of its atlas texels and skip.

#### 2. World-Position Reconstruction

```
ndc = vec3f(screen_uv * 2.0 - 1.0, depth)
view_h = proj_inv * vec4f(ndc, 1.0)
view_pos = view_h.xyz / view_h.w
world_pos = (view_inv * vec4f(view_pos, 1.0)).xyz
```

#### 3. Ray Interval Computation

```
t_start = select(0.0, f32(1u << (cascade_index - 1u)), cascade_index > 0u)
t_end   = select(1.0, f32(1u << cascade_index),        cascade_index > 0u)
```

| Cascade | Interval (voxels) |
|---|---|
| 0 | [0, 1] |
| 1 | [1, 2] |
| 2 | [2, 4] |
| 3 | [4, 8] |

Cascade 0 must start at `t=0` (Sannikov Eq. 18). Starting at any positive value leaves the near-field range unsampled.

#### 4. Per-Direction Ray March

For each direction in the probe's octahedral map (`2^i x 2^i` texels):

a. Decode octahedral texel `(ox, oy)` to world-space direction `vec3f`.

b. Call `traceSegments(probe_world_pos, direction, t_start, t_end)` as defined in the traversal contract.

c. Process returned segments:

```
var radiance = vec3f(0.0)
var opacity  = 0.0

for segment in interval_result:
    match segment:
        OpaqueSegment(t_enter, t_exit, voxel):
            slot = world_coord_to_slot(voxel >> 6)   // chunk coord
            local = voxel & 63
            mat_id = fetch_material_id(slot, local)
            emissive = emissive_lookup[mat_id].rgb * emissive_lookup[mat_id].a
            radiance += emissive
            opacity = 1.0
            break   // opaque: no further contribution in this interval

        EmptySegment(t_enter, t_exit):
            // no contribution -- traversal skipped empty region
```

d. Track transparency: if no opaque segment hit, `opacity = 0.0` (ray unblocked through interval); if hit, `opacity = 1.0`.

#### 5. Temporal Blend (Optional)

```
final = mix(fresh_radiance, reprojected_prev_radiance, temporal_alpha)
```

Where `temporal_alpha` is forced to `0.0` for probes whose rays intersect dirty chunks (edit invalidation).

#### 6. Atlas Write

Write `rgba16float` to `cascade_atlas[cascade_index]` at the probe's octahedral texel range:

```
atlas_x = px * 2^i + ox
atlas_y = py * 2^i + oy
cascade_atlas[cascade_index][atlas_y][atlas_x] = vec4f(radiance, opacity)
```

### DDA Traversal Path

Each ray follows the three-level DDA defined in the traversal contract:

**Level 0 -- Chunk DDA:** Ray steps through the chunk grid (64-voxel granularity). At each chunk:
- Not resident -> skip (treat as empty)
- `chunk_flags.is_empty` -> skip
- Otherwise -> descend to Level 1

**Level 1 -- Voxel DDA (inside chunk):** Ray steps through the 64x64x64 voxel grid. At each voxel:
- Test `chunk_occupancy_atlas` bit via column-major u64 access
- Hit -> emit `OpaqueSegment`, fetch material
- Exit chunk -> return to Level 0

**Bricklet skip (optional):** Between Level 0 and Level 1, test `occupancy_summary` bricklet bits (8x8x8 granularity) to skip empty sub-regions within non-empty chunks.

**Chunk skip for emissive:** `has_emissive` flag enables an optimization for emissive-only queries. However, non-emissive chunks still contribute opacity (they block light). Do not skip non-emissive chunks from opacity tracking.

---

## Outputs

| Buffer / Texture | Access | Format | What's written |
|---|---|---|---|
| `cascade_atlas[cascade_index]` | Write | `rgba16float` 2D, `screen_w x screen_h` | Per-probe radiance (`.rgb`) and opacity (`.a`) for each octahedral direction at this cascade level |

---

## Postconditions

| ID | Condition | Validates |
|---|---|---|
| POST-1 | Every active probe (non-far-plane depth) has radiance `.rgb >= 0.0` in all atlas texels | CAS-4 (non-negative radiance) |
| POST-2 | Every active probe has opacity `.a` in `[0.0, 1.0]` in all atlas texels | CAS-5 (opacity bounds) |
| POST-3 | Every inactive probe (far-plane depth) has `vec4f(0.0)` in all atlas texels | CAS-6 (inactive probe zeroing) |
| POST-4 | `cascade_atlas[i]` contains exactly `(screen_w / 2^i) x (screen_h / 2^i)` probes, each with `2^i x 2^i` texels | CAS-3 (layout correctness) |
| POST-5 | Each probe's rays traversed only the interval `[t_i, t_{i+1}]` -- no out-of-interval contributions | Interval discipline |
| POST-6 | Cascade 0 interval starts at `t=0` (contact shadows) | Sannikov Eq. 18 |
| POST-7 | No Hi-Z, frustum cull, or Product 3 data was used to filter traversal | Layer model: cascade build is Product 1 |

---

## Dispatch

```
Per cascade level i (dispatched highest to lowest):
  workgroup_size: (8, 8, 1)
  dispatch_x: ceil((screen_w / 2^i) / 8)
  dispatch_y: ceil((screen_h / 2^i) / 8)
  dispatch_z: 1
```

Total dispatches per frame: `N_CASCADES` (typically 4).

Each workgroup handles 64 probes. Each thread handles one probe and iterates over its `2^i x 2^i` octahedral directions sequentially.

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Probe placement math:** For known screen UV and depth, verify reconstructed world position matches expected value via inverse projection.
2. **Interval computation:** For each cascade index 0..3, verify `[t_start, t_end]` matches the reference table.
3. **Octahedral encode/decode roundtrip:** For 1000 random directions, verify `octahedral_decode(octahedral_encode(dir))` is within epsilon of the original.
4. **Atlas addressing:** For probe (px, py) in cascade i with octahedral texel (ox, oy), verify computed atlas coordinates are within `[0, screen_w) x [0, screen_h)`.

### GPU validation

5. **Single emissive voxel:** Place one emissive voxel at a known position. Dispatch cascade build. Readback cascade atlas. Verify probes whose rays reach that voxel contain non-zero radiance; probes whose rays miss contain zero radiance.
6. **Empty world:** Dispatch cascade build over an empty scene (no occupied chunks). Verify all atlas texels are zero.
7. **Opacity tracking:** Place an opaque non-emissive voxel between a probe and an emissive voxel. Verify the probe records opacity=1 but zero radiance (blocked).
8. **CPU-GPU agreement:** Run traversal on CPU reference implementation and GPU shader with identical inputs, compare per-probe results.

### Cross-stage tests

9. **R-6 -> R-7:** After cascade build, verify merge pass can read `cascade_atlas[i]` without NaN or negative values.
10. **Depth dependency:** Modify depth_texture (change camera), verify probe world positions change accordingly.
11. **I-3 -> R-6:** Chunks with `is_empty=1` are never descended into during DDA traversal (verify via diagnostic counter `chunks_empty_skipped`).

---

## See Also

- [traversal-acceleration](../traversal-acceleration.md) -- `traceSegments` contract; three-level DDA design
- [radiance-cascades-impl](../radiance-cascades-impl.md) -- full cascade build algorithm, temporal reprojection, shader file layout
- [pipeline-stages](../pipeline-stages.md) -- R-6 buffer ownership and stage ordering
- [cascade-atlas](../data/cascade-atlas.md) -- output texture layout, invariants, memory budget
- [depth-texture](../data/depth-texture.md) -- input depth buffer consumed for probe placement
- [chunk-occupancy-atlas](../data/chunk-occupancy-atlas.md) -- world-space occupancy data traversed by rays
