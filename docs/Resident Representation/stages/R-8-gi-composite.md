# Stage R-8: GI Application + Composite

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Stage type:** UNDERSPECIFIED -- inline in R-5 fragment shader OR separate fullscreen pass (see Variants below)
**Trigger:** Every frame, after R-7 (cascade merge). Reads merged cascade_atlas_0 and applies GI contribution to final color.

> Samples the merged radiance cascade and composites global illumination into the rendered image. The narrative docs are ambiguous on whether this is an inline texture sample in R-5 or a separate fullscreen pass. Both variants are documented here.

---

## Purpose

After R-7 merges all cascade levels into `cascade_atlas_0`, the GI radiance must be applied to the rendered scene. This stage reads the merged cascade 0 data and integrates it as a diffuse irradiance term on each visible surface fragment. The result is indirect lighting -- surfaces illuminated by light bouncing off emissive voxels, including contact shadows in the near-field (cascade 0 interval `[0, 1]`).

---

## UNDERSPECIFIED: Inline vs. Separate Pass

The narrative docs describe R-8 inconsistently:

- [pipeline-stages](../pipeline-stages.md) (Stage R-8 section) states: "Handled inline in R-5 fragment shader or as a separate fullscreen pass."
- [radiance-cascades-impl](../radiance-cascades-impl.md) (Stage R-5 / R-8 section) describes only the inline variant, showing `cascade_atlas[0]` sampled directly in the R-5 fragment shader.
- [pipeline-stages](../pipeline-stages.md) (Stage R-5 section) lists `cascade_atlas_0` as a READ input to R-5 and notes "Fragment shader integrates hemisphere from cascade 0 for diffuse GI."

**Both variants are specified below.** The choice affects pipeline ordering, barrier requirements, and shader complexity but not the GI math itself. A decision is needed before implementation.

---

## Variant A: Inline in R-5 Fragment Shader

### Description

No separate R-8 pass exists. The R-5 main color pass fragment shader reads `cascade_atlas_0` as an additional texture input and adds the GI term directly to the fragment color output. R-8 is logically part of R-5.

### Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-A1 | `cascade_atlas_0` contains fully merged radiance from R-7 | R-7 postcondition |
| PRE-A2 | R-7 completes before R-5 begins (explicit barrier or separate submit) | Pipeline ordering |

**Ordering constraint:** R-5 reads `cascade_atlas_0`, which is written by R-7. R-7 must therefore complete before R-5 begins. This means the stage order is effectively: R-6 -> R-7 -> R-5 (not R-5 -> R-6 -> R-7 -> R-8). The pipeline-stages diagram shows R-5 before R-6, but with inline GI, R-5 must wait for R-7. This is a scheduling tension that needs resolution.

**Alternative:** Use previous frame's merged cascade. R-5 reads `cascade_atlas_prev_0` (one frame latent). This allows R-5 to run before R-6/R-7 in the current frame, at the cost of one frame of GI latency. The temporal lag is generally acceptable for indirect lighting.

### Inputs (Variant A)

| Buffer / Texture | Access | Format | What's read |
|---|---|---|---|
| `cascade_atlas_0` (or `cascade_atlas_prev_0`) | Read | `rgba16float` 2D | Merged radiance at cascade 0 probe positions |
| `material_table` | Read | `array<MaterialEntry>` | Albedo for diffuse GI multiplication |

(All other R-5 inputs -- `vertex_pool`, `index_pool`, `indirect_draw_buf`, `depth_texture` -- are unchanged.)

### Transformation (Variant A)

In the R-5 fragment shader, after computing direct lighting:

```wgsl
// GI term from cascade 0
let screen_uv = fragment_position.xy / screen_size;

// For cascade 0 (1x1 per probe), the probe IS the texel
let probe_radiance = textureSample(cascade_atlas_0, cascade_sampler, screen_uv);

// Diffuse GI: irradiance * albedo * 1/pi
let diffuse_gi = probe_radiance.rgb * material_albedo * INV_PI;

// Add GI to direct lighting
final_color = direct_lighting + diffuse_gi;
```

For cascade 0 with 1x1 octahedral map per probe, each probe stores a single radiance value (scalar irradiance). No directional interpolation is needed. For higher-resolution per-probe maps (future), sample the octahedral map in the hemisphere aligned with the surface normal.

### Outputs (Variant A)

| Buffer / Texture | Access | Format | What's written |
|---|---|---|---|
| `color_target` | Write | `rgba8unorm` | Final color including both direct and indirect (GI) lighting |

---

## Variant B: Separate Fullscreen Pass

### Description

R-8 is a distinct fullscreen pass that runs after R-5. R-5 writes direct lighting only. R-8 reads the R-5 color output and `cascade_atlas_0`, composites GI, and writes the final color.

### Preconditions

| ID | Condition | Source |
|---|---|---|
| PRE-B1 | `cascade_atlas_0` contains fully merged radiance from R-7 | R-7 postcondition |
| PRE-B2 | `color_target` contains direct lighting output from R-5 | R-5 postcondition |
| PRE-B3 | `depth_texture` contains valid depth (for depth-aware GI sampling if needed) | R-2 postcondition |
| PRE-B4 | R-5 and R-7 both complete before R-8 begins | Pipeline ordering |

### Inputs (Variant B)

| Buffer / Texture | Access | Format | What's read |
|---|---|---|---|
| `color_target` | Read | `rgba8unorm` | Direct lighting from R-5 |
| `cascade_atlas_0` | Read | `rgba16float` 2D | Merged radiance from R-7 |
| `depth_texture` | Read | `depth32float` | For reconstructing surface properties if needed |
| `material_table` | Read | `array<MaterialEntry>` | Albedo for diffuse multiplication (requires per-pixel material ID or G-buffer) |

### Transformation (Variant B)

Fullscreen quad (or fullscreen compute dispatch):

```wgsl
@fragment
fn fs_main(@builtin(position) frag_pos: vec4f) -> @location(0) vec4f {
    let screen_uv = frag_pos.xy / screen_size;

    // Read direct lighting from R-5
    let direct = textureSample(color_target, sampler, screen_uv);

    // Read merged cascade 0 GI
    let probe_radiance = textureSample(cascade_atlas_0, cascade_sampler, screen_uv);

    // Composite: direct + indirect
    // NOTE: albedo multiplication requires per-pixel material info.
    // If no G-buffer exists, GI is applied as a simple additive/modulated term.
    let gi_contribution = probe_radiance.rgb * gi_intensity_scale;
    let final_color = direct.rgb + gi_contribution;

    return vec4f(final_color, direct.a);
}
```

**Limitation of Variant B:** Without a G-buffer, the separate pass lacks per-pixel albedo for physically correct `irradiance * albedo * 1/pi` multiplication. The GI contribution must either be applied as a flat additive term (less correct) or a G-buffer must be added to the pipeline (additional memory and bandwidth cost).

### Outputs (Variant B)

| Buffer / Texture | Access | Format | What's written |
|---|---|---|---|
| `color_target` | Write | `rgba8unorm` | Final composited color (direct + indirect lighting) |

### Dispatch (Variant B)

```
Fullscreen render pass:
  Single triangle or quad covering the viewport
  Fragment shader samples color_target + cascade_atlas_0

OR fullscreen compute:
  workgroup_size: (8, 8, 1)
  dispatch_x: ceil(screen_w / 8)
  dispatch_y: ceil(screen_h / 8)
  dispatch_z: 1
```

---

## Variant Comparison

| Aspect | Variant A (inline) | Variant B (separate) |
|---|---|---|
| Pass count | 0 extra passes | 1 fullscreen pass |
| Bandwidth | Lower -- no extra read/write of color_target | Higher -- reads and rewrites color_target |
| Albedo access | Has per-fragment material data naturally | Needs G-buffer or loses per-pixel albedo |
| Pipeline ordering | R-7 must complete before R-5 (or use prev-frame cascade) | R-5 and R-7 can run in parallel; R-8 waits for both |
| Shader complexity | R-5 fragment shader gains cascade sampling logic | R-5 stays simpler; R-8 is a self-contained pass |
| Flexibility | Tightly coupled to R-5 | Can be disabled/swapped independently |

---

## Postconditions (Both Variants)

| ID | Condition | Validates |
|---|---|---|
| POST-1 | `color_target` contains direct + indirect lighting for all visible fragments | GI application completeness |
| POST-2 | No NaN or negative values in final color output | Value sanity |
| POST-3 | Fragments with zero GI contribution (inactive probes, no emissive in range) have unchanged direct lighting | GI is purely additive |
| POST-4 | GI contribution is non-negative (indirect light adds, never subtracts) | Physical correctness |

---

## Testing Strategy

### Unit tests (Rust / TypeScript, CPU-side)

1. **GI math:** Verify `irradiance * albedo * INV_PI` produces expected diffuse GI for known radiance and albedo values.
2. **Zero GI passthrough:** With zero cascade radiance, verify final color equals direct lighting exactly.
3. **Screen UV mapping:** Verify probe sampling coordinates correctly map fragment positions to cascade 0 probe locations.

### GPU validation

4. **Emissive scene:** Render a scene with one emissive voxel and surrounding non-emissive surfaces. Verify surfaces near the emissive voxel receive non-zero GI contribution.
5. **No emissive baseline:** Render a scene with no emissive materials. Verify GI contribution is zero everywhere and final color matches R-5 direct-only output.
6. **Variant parity:** If both variants are implemented, verify they produce identical final color output for the same inputs (within floating-point tolerance).

### Cross-stage tests

7. **R-7 -> R-8:** Verify merged cascade 0 data is consumed correctly (no format mismatch, no stale binding).
8. **R-8 -> R-9:** Verify debug visualization can read the composited color_target after R-8 writes it.
9. **Previous-frame latency:** If using previous-frame cascade, verify one-frame delay is visually acceptable for moving emissive sources.

---

## See Also

- [radiance-cascades-impl](../radiance-cascades-impl.md) -- GI application algorithm, cascade 0 sampling, hemisphere integration
- [pipeline-stages](../pipeline-stages.md) -- R-8 stage definition and buffer ownership
- [cascade-atlas](../data/cascade-atlas.md) -- merged cascade 0 layout and invariants
- [R-7-cascade-merge](R-7-cascade-merge.md) -- upstream stage that produces merged cascade_atlas_0
- [R-6-cascade-build](R-6-cascade-build.md) -- raw cascade data production
