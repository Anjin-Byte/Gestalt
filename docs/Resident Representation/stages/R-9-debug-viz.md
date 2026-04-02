# Stage R-9: Viewport Shading & Debug Visualization

**Type:** spec
**Status:** current
**Date:** 2026-03-23
**Stage type:** GPU render/compute (conditional — behavior depends on active render mode)
**Trigger:** Every frame, last stage. Controlled by `SetRenderMode` command from the main thread.

> Controls how the final image is presented. Working modes (Solid, GI, Wireframe) are for everyday use. Debug modes (Depth, Hi-Z, Chunk State, Occupancy) expose pipeline internals. All modes run GPU-side in the worker.

---

## Purpose

The viewport needs multiple shading modes for different tasks — the same geometry viewed differently depending on what the user is doing. An artist evaluating surface shape wants Solid. A developer diagnosing culling wants Hi-Z Mip. A lighting artist wants GI. These are not "debug" features bolted on top — they are first-class viewport modes, each with a specific use case.

All modes reuse existing GPU buffers. No new GPU data is allocated per mode. Mode changes take effect next frame — no pipeline recreation needed if all pipelines are pre-created at init.

---

## Mode Registry

Modes are split into two ranges in the protocol:

- **`0x00–0x0F`**: Working views — everyday shading for geometry and lighting work
- **`0x10–0x1F`**: Debug views — GPU pipeline diagnostic visualization

### Working Views

| Code | Name | Replaces color? | Use case |
|---|---|---|---|
| `0x00` | **Solid** | Yes | Flat directional light, no GI. Default working mode. Fast, clear geometry reads. |
| `0x01` | **GI** | Yes | Full cascade GI (beauty mode). Active when cascades are wired. |
| `0x02` | **Wireframe** | Yes | Edges only, transparent faces. See through geometry, check topology. |
| `0x03` | **Solid + Wireframe** | Composite | Solid shading with wireframe edge overlay. Shape and topology simultaneously. |
| `0x04` | **Normals** | Yes | World-space normal → RGB. Debugging surface orientation, finding flipped faces. |
| `0x05` | **Matcap** | Yes | Spherical environment material. Quick surface quality check without lighting setup. |

### Debug Views

| Code | Name | Replaces color? | Use case |
|---|---|---|---|
| `0x10` | **Depth** | Yes | Grayscale linearized depth. Verify R-2 output, check depth distribution. |
| `0x11` | **Hi-Z Mip** | Yes | Selected mip level of Hi-Z pyramid. Verify R-3, diagnose occlusion cull. |
| `0x12` | **Chunk State** | Yes | Color chunks by lifecycle state (clean/dirty/empty/emissive). |
| `0x13` | **Occupancy** | Yes | Bricklet density heatmap. Hot = dense, cold = sparse. Affects traversal cost. |

---

## Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-1 | `color_target` contains R-5/R-8 output (used by composite modes and as GI base) | R-5/R-8 postcondition |
| PRE-2 | `depth_texture` contains valid depth from R-2 | R-2 postcondition |
| PRE-3 | `hiz_pyramid` contains valid mip chain from R-3 (debug view 0x11 only) | R-3 postcondition |
| PRE-4 | `chunk_flags` contains valid state bits from I-3 (debug view 0x12 only) | I-3 postcondition |
| PRE-5 | `occupancy_summary` contains valid bricklet bits from I-3 (debug view 0x13 only) | I-3 postcondition |
| PRE-6 | `render_mode` uniform set via `SetRenderMode` command | Main thread protocol |
| PRE-7 | R-9 must not write to `depth_texture` | DT-6 |

---

## Inputs

| Buffer / Texture | Access | Modes that read it |
|---|---|---|
| `depth_texture` | Read | All modes (depth test, linearization, world-position reconstruction) |
| `color_target` | Read | Solid+Wireframe (0x03) composite base |
| `hiz_pyramid` | Read | Hi-Z Mip (0x11) |
| `chunk_flags` | Read | Chunk State (0x12) |
| `occupancy_summary` | Read | Occupancy (0x13) |
| `vertex_pool` | Read | Wireframe (0x02), Solid+Wireframe (0x03), Normals (0x04), Chunk State (0x12) |
| `index_pool` | Read | Same as vertex_pool |
| `draw_metadata` | Read | Same as vertex_pool |
| `camera_uniform` | Read | All modes with 3D rendering |
| `matcap_texture` | Read | Matcap (0x05) — a 256×256 spherical environment map, loaded once at init |

---

## Mode Specifications

### 0x00 — Solid

The default working mode. Simple directional lighting with no GI, no textures, no environment. The goal is maximum geometric clarity.

```wgsl
let light_dir = normalize(vec3f(0.3, 1.0, 0.5));
let ndotl = max(dot(normal, light_dir), 0.0);
let color = material_albedo * (0.15 + 0.85 * ndotl);
```

**Implementation:** R-5 renders with its standard pipeline but skips cascade atlas sampling. The `render_mode` uniform gates the GI term in the fragment shader. When `render_mode == 0x00`, the cascade sample is replaced by a fixed ambient of 0.15.

This mode is always available — it doesn't depend on cascades being built.

### 0x01 — GI

Full radiance cascade GI. R-5 renders with cascade atlas sampling enabled. This is the beauty mode — what the final output looks like.

**Implementation:** R-5's standard path with `render_mode == 0x01`. Fragment shader samples `cascade_atlas_0` and applies `albedo * (ambient + gi_irradiance)`. Only available when cascades are wired (Phase 3+). Before Phase 3, this mode falls back to Solid.

### 0x02 — Wireframe

Edges only, transparent faces. For seeing through geometry and checking quad density from the greedy mesher.

**Implementation:** Re-render chunk geometry with `topology: line-list`. Generate line indices from triangle indices (each triangle → 3 edges, deduplicate shared edges). Depth test enabled (read-only) so wireframe correctly occludes against itself. Single color (white or configurable).

**Alternative (simpler):** Use `topology: triangle-list` with a fragment shader that outputs color only near triangle edges using screen-space derivatives:
```wgsl
let bary = fwidth(barycentric);
let edge = smoothstep(0.0, bary * 1.5, min(min(barycentric.x, barycentric.y), barycentric.z));
color = mix(wire_color, vec4f(0.0), edge);
```

### 0x03 — Solid + Wireframe

Solid shading with wireframe edge overlay. The most used debug view in DCC tools.

**Implementation:** R-5 runs normally in Solid mode → `color_target` has solid-shaded output. R-9 then renders wireframe on top with `loadOp: load` and additive or alpha blending. The wireframe lines are drawn with depth test (read-only) against the existing depth buffer.

### 0x04 — Normals

World-space normal → RGB: `color = normal * 0.5 + 0.5`. For debugging surface orientation and finding flipped faces.

**Implementation:** Re-render chunk geometry with a fragment shader that outputs the interpolated world-space normal as color. Uses the same vertex buffer (normals are packed in the vertex data).

### 0x05 — Matcap

Material capture — a spherical environment texture applied based on view-space normals. For quick surface quality evaluation without setting up lights.

**Implementation:** Re-render chunk geometry. Fragment shader transforms the world normal to view space, uses the XY components as UV to sample a 256×256 matcap texture. The matcap is a pre-loaded 2D texture (loaded at init from a built-in asset or user-selected).

```wgsl
let view_normal = (camera_uniform.view * vec4f(normal, 0.0)).xyz;
let uv = view_normal.xy * 0.5 + 0.5;
color = textureSample(matcap_texture, matcap_sampler, uv);
```

### 0x10 — Depth

Grayscale linearized depth. Near = white, far = black.

**Implementation:** Fullscreen pass. Reads `depth_texture`, linearizes, maps to grayscale.

```wgsl
let linear_depth = camera_uniform.near / depth_value;
let normalized = saturate(linear_depth / far_viz_range);
color = vec4f(vec3f(normalized), 1.0);
```

### 0x11 — Hi-Z Mip

Selected mip level of the Hi-Z pyramid. Controlled by `debug_params.hiz_mip_level`.

**Implementation:** Fullscreen pass. Reads `hiz_pyramid` at the selected mip, maps depth to grayscale. Lower mip levels show the conservative max-depth used by R-4 for occlusion decisions.

### 0x12 — Chunk State

Color each chunk by lifecycle state.

| State | Color | Flags |
|---|---|---|
| Clean (resident, not empty, not stale) | Green | `is_resident=1, is_empty=0, stale_mesh=0` |
| Dirty (stale mesh or summary) | Yellow | `stale_mesh=1` or `stale_summary=1` |
| Empty | Dark blue | `is_empty=1` |
| Emissive | Orange | `has_emissive=1` |
| Non-resident | Red | `is_resident=0` |

**Implementation:** Re-render chunk geometry. Fragment shader reads `chunk_flags[slot]` and selects color.

### 0x13 — Occupancy Heatmap

Bricklet density visualization. Hot (red) = dense, cold (blue) = sparse. Reveals spatial distribution that directly affects traversal cost in R-6.

**Implementation:** Fullscreen pass. For each pixel, reconstruct world position from depth, determine chunk + bricklet, count occupied bricklets in the neighborhood from `occupancy_summary`, map to heat colormap.

---

## Outputs

| Buffer / Texture | Access | What's written |
|---|---|---|
| `color_target` | Write (replace or composite) | Final viewport image |

- **Replace modes** (Solid, GI, Wireframe, Normals, Matcap, all debug views): overwrite `color_target` entirely.
- **Composite modes** (Solid+Wireframe): wireframe blended over Solid output.

---

## Postconditions

| ID | Condition | Validates |
|---|---|---|
| POST-1 | `depth_texture` is unmodified (R-9 reads but never writes depth) | DT-6 |
| POST-2 | `color_target` contains the visualization for the active render mode | Mode correctness |
| POST-3 | Switching from any mode to any other mode produces correct output on the next frame | No stale state between modes |
| POST-4 | All visualization is GPU-side in the worker — no DOM rendering | Architecture constraint |

---

## Dispatch

| Mode | Dispatch type |
|---|---|
| 0x00 Solid | Handled by R-5 fragment shader (no separate R-9 dispatch) |
| 0x01 GI | Handled by R-5 fragment shader (no separate R-9 dispatch) |
| 0x02 Wireframe | Render pass: re-render with line topology or edge-detect fragment |
| 0x03 Solid+Wire | R-5 runs Solid → R-9 renders wireframe overlay |
| 0x04 Normals | Render pass: re-render with normal-color fragment |
| 0x05 Matcap | Render pass: re-render with matcap-sample fragment |
| 0x10 Depth | Fullscreen compute: `ceil(w/8) × ceil(h/8) × 1` |
| 0x11 Hi-Z Mip | Fullscreen compute: `ceil(w/8) × ceil(h/8) × 1` |
| 0x12 Chunk State | Render pass: re-render with state-color fragment |
| 0x13 Occupancy | Fullscreen compute: `ceil(w/8) × ceil(h/8) × 1` |

---

## Testing Strategy

### Unit tests

1. **Mode enum coverage:** All documented mode values are handled — no undefined behavior for any valid code.
2. **Protocol roundtrip:** `SetRenderMode` encodes/decodes correctly. Mode persists across frames until changed.
3. **Debug params:** Hi-Z mip level clamped to valid range. Matcap texture index valid.

### GPU validation

4. **Solid = R-5 without GI:** Render a scene in Solid mode and in GI mode with cascades zeroed. Output must match.
5. **Depth accuracy:** Render known geometry at known depth. Verify grayscale value at that pixel matches expected linearized depth within f32 tolerance.
6. **Chunk state correctness:** Load a scene with empty and non-empty chunks. Verify empty = blue, occupied = green.
7. **Depth preservation:** After any mode, readback `depth_texture` and verify it's unchanged from post-R-2.
8. **Mode switching:** Cycle through all modes rapidly (every frame). No crashes, no stale artifacts, no GPU validation errors.

### Cross-stage tests

9. **R-5 → R-9 wireframe:** Wireframe composites correctly over R-5 color without z-fighting or bleed.
10. **R-3 → R-9 Hi-Z:** Hi-Z mip visualization shows correct pyramid (each level half resolution of previous, max-reduction).
11. **I-3 → R-9 occupancy:** Heatmap reflects actual bricklet occupancy from `occupancy_summary`.
12. **Normals vs geometry:** Normal visualization colors match expected face orientations for axis-aligned geometry (±X = red/cyan, ±Y = green/magenta, ±Z = blue/yellow).

---

## See Also

- [debug-profiling](../debug-profiling.md) — diagnostic counters, testing strategy
- [pipeline-stages](../pipeline-stages.md) — R-9 stage definition, buffer ownership
- [depth-texture](../data/depth-texture.md) — DT-6: R-9 must not write depth
- [R-5 color pass](R-5-color-pass.md) — Solid and GI modes are R-5 fragment shader variants
