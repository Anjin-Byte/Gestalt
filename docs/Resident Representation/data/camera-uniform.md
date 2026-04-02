# Camera Uniform

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Per-frame transient — written by the CPU (Rust camera module) every frame.

> All camera-derived matrices and vectors needed by render and compute stages. Single buffer, single binding, read by everything.

---

## Identity

- **Buffer name:** `camera_uniform`
- **WGSL type:** `struct CameraUniforms` (uniform buffer)
- **GPU usage:** `UNIFORM | COPY_DST`
- **Binding:** `@group(0) @binding(N)` — shared across all render and compute stages
- **Size:** 256 bytes (padded to `minUniformBufferOffsetAlignment`)

---

## Layout

```wgsl
struct CameraUniforms {
  view:          mat4x4f,   // bytes  0..63   — world → view
  proj:          mat4x4f,   // bytes 64..127  — view → clip
  view_proj:     mat4x4f,   // bytes 128..191 — world → clip (precomputed: proj * view)
  camera_pos:    vec4f,     // bytes 192..207 — world-space camera position (.w = 0)
  camera_forward: vec4f,    // bytes 208..223 — world-space forward direction (.w = 0)
  viewport_size: vec2f,     // bytes 224..231 — (width, height) in pixels
  near:          f32,       // bytes 232..235
  far:           f32,       // bytes 236..239
  frame_index:   u32,       // bytes 240..243 — monotonic frame counter (for temporal reprojection)
  _pad:          array<u32, 3>, // bytes 244..255 — padding to 256
};
```

Total: **256 bytes**. One buffer, not per-slot.

All matrices are **column-major** (WebGPU/WGSL convention). `mat4x4f` is 4 columns × 4 rows × 4 bytes = 64 bytes.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| CAM-1 | `view_proj == proj * view` (exact — no accumulated error) | Computed once per frame, not incrementally |
| CAM-2 | `camera_pos.w == 0` and `camera_forward.w == 0` | Camera module sets `.w = 0` explicitly |
| CAM-3 | `camera_forward` is normalized (`length ≈ 1.0 ± 1e-6`) | Camera module normalizes |
| CAM-4 | `viewport_size.x > 0 && viewport_size.y > 0` | ResizeViewport command clamps to ≥ 1 |
| CAM-5 | `near > 0 && near < far` | Camera module enforces |
| CAM-6 | `frame_index` increments by exactly 1 each frame | Frame loop |
| CAM-7 | `proj` is a valid perspective matrix (finite, non-zero determinant) | Camera module constructs from FOV + aspect + near + far |

---

## Producers

| Producer | When |
|---|---|
| Rust camera module | Every frame, before any render/compute pass |

The camera module receives `SetCamera(pos, dir)` and `ResizeViewport(w, h)` commands from the main thread via the binary protocol. It computes the matrices and writes the buffer via `queue.writeBuffer`.

---

## Consumers

| Consumer | Stage | Fields read |
|---|---|---|
| Depth prepass | R-2 | `view_proj` |
| Hi-Z build | R-3 | `viewport_size` (for mip level count) |
| Occlusion cull | R-4 | `view_proj`, `camera_pos`, `viewport_size`, `near`, `far` |
| Color pass | R-5 | `view_proj`, `camera_pos`, `camera_forward` |
| Cascade build | R-6 | `view`, `proj`, `camera_pos`, `viewport_size`, `near` |
| Cascade merge | R-7 | `viewport_size` |
| Debug viz | R-9 | `view_proj`, `near`, `far` (for depth linearization) |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Matrix correctness:** For known camera position + direction, verify `view` matches a reference lookAt implementation.
2. **Projection correctness:** For known FOV + aspect + near + far, verify `proj` matches a reference perspective implementation.
3. **view_proj composition:** Verify `view_proj == proj * view` to within f32 epsilon.
4. **Normalization:** After any `SetCamera`, verify `length(camera_forward) ∈ [1.0 - 1e-6, 1.0 + 1e-6]`.
5. **Near/far validity:** Verify `near > 0 && near < far` for all valid SetCamera inputs.

### Property tests (Rust, randomized)

6. **Inverse consistency:** For random camera params, verify `view * inverse(view) ≈ identity`.
7. **Clip space bounds:** For a point at `camera_pos + camera_forward * 1.0`, verify it projects to clip Z ∈ (0, 1).
8. **Frame index monotonicity:** Over 1000 frames, verify `frame_index` strictly increases by 1.

### Integration tests

9. **Pipeline read:** Every stage that reads `camera_uniform` receives the same values within a frame (no mid-frame mutation).
