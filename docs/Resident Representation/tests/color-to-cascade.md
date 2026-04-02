# Test: Color to Cascade (R-5 → R-6)

**Type:** spec
**Status:** current
**Date:** 2026-03-22

> Proves the color pass output is valid cascade-build input: depth_texture is unchanged after R-5 (depthWriteEnabled=false preserves R-2 depth), cascade probes can read valid depth values, sky probes are handled correctly, and world-space reconstruction from depth is accurate.

---

## What This Tests

The color-to-cascade chain connects rasterization to global illumination:

```
depth_texture (R-2, preserved through R-5) → probe placement + world-position reconstruction (R-6)
```

R-5 binds depth_texture for depth testing but does not write to it (`depthWriteEnabled: false`). R-6 reads depth_texture to place probes at surface positions and reconstruct world-space coordinates. If R-5 inadvertently modifies depth, or if depth reconstruction is inaccurate, cascade probes will be misplaced and the radiance field will be incorrect. This document defines the tests that prove the handoff is correct.

---

## Chain Link 1: R-5 Depth Preservation

**Claim:** `depth_texture` is identical before and after R-5 — R-5 reads depth for testing but does not write it.

### Preconditions (R-6 input contract — depth)

| ID | What R-6 requires | How R-5 must satisfy it |
|---|---|---|
| L1-1 | `depth_texture` contains the same depth values written by R-2 | R-5 POST-2: depthWriteEnabled=false; depth is unmodified |
| L1-2 | `depth_texture` format is `depth32float` with `TEXTURE_BINDING` usage | R-2 POST-3: created with correct usage flags |
| L1-3 | Depth values are in [0, 1] for standard Z | R-2 guarantee (validated by depth-to-cull T-L1-1) |

### Tests

```
T-L1-1: Depth unchanged after R-5
  Run R-2 (depth prepass)
  Read back depth_texture → snapshot_before
  Run R-5 (color pass with depthWriteEnabled=false)
  Read back depth_texture → snapshot_after
  For each texel (x, y):
    Assert: snapshot_before[x][y] == snapshot_after[x][y]   (bitwise equal)

T-L1-2: Pipeline configuration audit
  Inspect R-5 render pipeline descriptor:
    Assert: depthStencil.depthWriteEnabled == false
  Inspect R-5 render pass descriptor:
    Assert: depthStencilAttachment.depthLoadOp == 'load'  (preserve R-2 depth)
    Assert: depthStencilAttachment.depthStoreOp == 'store' (keep for downstream)

T-L1-3: Depth survives multiple R-5 invocations
  Run R-2 once
  Run R-5 three times consecutively (simulating multi-pass scenarios)
  Read back depth_texture
  Assert: values match the single R-2 output (no accumulation or drift)
```

---

## Chain Link 2: Probe Placement from Depth

**Claim:** R-6 probes placed via depth sampling read valid depth values and correctly identify surface vs. sky.

### Tests

```
T-L2-1: Probes at geometry surfaces read valid depth
  Place known geometry at world position P
  Run R-2 (populates depth)
  For each probe whose screen footprint overlaps the geometry:
    Sample depth_texture at probe's screen UV
    Assert: depth != depthClearValue (1.0 for standard Z)
    Assert: depth is finite and in [0, 1)
    Assert: the probe is marked active (not sky)

T-L2-2: Probes at sky (depth=1.0) are skipped
  Place the camera looking at empty sky (no geometry in view)
  Run R-2 (all texels are clear value = 1.0)
  Run R-6
  Read back cascade_atlas for cascade level 0
  For each probe:
    Assert: all octahedral texels are vec4f(0.0)
    (R-6 POST-3: inactive probes produce zero radiance and zero opacity)

T-L2-3: Mixed surface and sky probes
  Place geometry covering half the screen
  Run R-2, then R-6
  Read back cascade_atlas
  For probes over geometry:
    Assert: at least some texels have non-zero opacity (probes are active and tracing)
  For probes over sky:
    Assert: all texels are vec4f(0.0)

T-L2-4: Depth sampling at probe center
  For cascade level i, probe (px, py):
    Expected screen pixel = (px * 2^i + 2^(i-1), py * 2^i + 2^(i-1))
    Place a thin horizontal surface at a known depth
    Verify the probe samples depth at that exact pixel location
    Assert: sampled depth matches the surface's expected NDC depth
```

---

## Chain Link 3: World-Space Reconstruction Accuracy

**Claim:** World positions reconstructed from depth via inverse projection match the known geometry positions within tolerance.

### Tests

```
T-L3-1: Reconstruction roundtrip — known geometry
  Place a voxel surface at known world position W = (10.5, 20.5, 30.5)
  Run R-2 (rasterize depth)
  Compute the screen UV where W projects
  Sample depth at that UV → d
  Reconstruct world position:
    ndc = vec3f(uv * 2.0 - 1.0, d)
    view_h = proj_inv * vec4f(ndc, 1.0)
    view_pos = view_h.xyz / view_h.w
    world_reconstructed = (view_inv * vec4f(view_pos, 1.0)).xyz
  Assert: |world_reconstructed - W| < tolerance (0.1 voxel units)

T-L3-2: Reconstruction at screen corners
  Place geometry at all four screen corners
  For each corner:
    Run reconstruction
    Assert: reconstructed world position matches expected position within tolerance
  (Screen corners have the highest projection distortion — if corners are
   accurate, center positions will be more accurate.)

T-L3-3: Reconstruction at varying depths
  Place surfaces at 5 different known distances (near, mid-near, mid, mid-far, far)
  For each surface:
    Reconstruct world position from depth
    Assert: reconstructed Z (depth axis) matches known distance within tolerance
  (Depth precision degrades with distance in standard Z. Verify tolerance
   scales appropriately or switch to reversed-Z.)

T-L3-4: Reconstruction consistency with camera movement
  Place a static surface at world position W
  For 10 different camera positions:
    Run R-2
    Reconstruct world position from depth at the surface's projected UV
    Assert: reconstructed position == W within tolerance (0.1 voxel units)
  (World position is camera-independent — reconstruction must be stable.)

T-L3-5: Reconstruction at depth=1.0 (sky)
  Sample depth = 1.0 (far plane)
  Reconstruct world position
  Assert: the reconstructed position is at or near the far plane distance
  Assert: R-6 skips this probe (does not use the sky position for tracing)
  (R-6 POST-3: inactive probes zeroed. The reconstruction itself may produce
   a valid far-plane position, but it must not feed into radiance computation.)
```

---

## Full Chain Integration Test

```
T-FULL-1: Color → Cascade with known emissive
  Input: one emissive voxel (red, intensity 2.0) at known world position E
         one opaque surface at known world position S, facing the emissive voxel
  Run R-2 (depth for surface S)
  Run R-5 (color for surface S — placeholder lighting)
  Run R-6 (cascade build)

  For probes placed on surface S:
    Reconstruct world position from depth
    Assert: world position matches S within tolerance
    For octahedral directions pointing toward E:
      Assert: radiance.rgb > 0 (emissive contribution detected)
      Assert: opacity == 1.0 (ray hit opaque emissive voxel)
    For octahedral directions pointing away from E:
      Assert: radiance == 0 or opacity == 0 (no emissive in that direction)

  For probes placed on sky pixels:
    Assert: all texels are vec4f(0.0)
```

---

## Consistency Properties (Hold for Any Valid R-5 → R-6 Transition)

```
P-1: depth_texture after R-5 is bitwise identical to depth_texture after R-2
  (R-5 POST-2: depthWriteEnabled=false)

P-2: For every active probe (depth != far-plane clear value):
  Reconstructed world position is finite (not NaN, not Inf)
  Reconstructed world position is within the scene's world bounds

P-3: For every inactive probe (depth == far-plane clear value):
  All cascade_atlas texels for this probe are vec4f(0.0)
  (R-6 POST-3)

P-4: For every active probe:
  radiance.rgb >= 0.0 in all texels (R-6 POST-1)
  opacity in [0.0, 1.0] in all texels (R-6 POST-2)

P-5: World-space reconstruction from depth is invertible:
  project(reconstruct(depth, uv)) == (uv, depth) within floating-point tolerance
```

These properties bridge R-5 postconditions (POST-1 through POST-5) and R-6 preconditions (PRE-1 through PRE-8).

---

## See Also

- [R-5-color-pass](../stages/R-5-color-pass.md) -- producer: renders color, preserves depth
- [R-6-cascade-build](../stages/R-6-cascade-build.md) -- consumer: reads depth for probe placement and world reconstruction
- [depth-texture](../data/depth-texture.md) -- shared depth buffer (R-2 writes, R-5 tests, R-6 reads)
- [cascade-atlas](../data/cascade-atlas.md) -- output texture written by R-6
- [radiance-cascades-impl](../radiance-cascades-impl.md) -- full cascade algorithm and probe placement details
