// Half-res sun-shadow pass — the "low" shadow-quality tier. Concatenated after
// traversal.wgsl (structure bindings + traverse_ray in scope). Dispatched at
// ½×½ resolution: each lo-res texel samples the full-res G-buffer at its source
// pixel (2·coord), reconstructs the hit, and traces the EXACT per-voxel shadow
// ray — a quarter of the rays for correct-shaped umbras, softened by composite's
// joint-bilateral upsample. When this tier is active (dims.w bit5) the G-buffer
// skips its own inline shadow ray, so this pass is the sole shadow trace.
//
// Simplified from the GTAO branch's reference: fine trace only (no coarse
// hybrid — the brick-level trace reads fragmented, which is what this tier
// replaces) and no per-frame source jitter (a dropped experiment; the
// anisotropic upsample handles the spatial aliasing).

struct Camera {
    eye: vec3<f32>,
    tan: f32,
    forward: vec3<f32>,
    aspect: f32,
    right: vec3<f32>,
    n: f32,
    up: vec3<f32>,
    _pad: f32,
    dims: vec4<u32>,
}

@group(0) @binding(3) var<uniform> camera: Camera;
@group(0) @binding(4) var gbuf_depth: texture_2d<f32>;  // full-res
@group(0) @binding(5) var gbuf_normal: texture_2d<f32>; // full-res (normal in .xyz)
@group(0) @binding(6) var<uniform> sun_dir: vec4<f32>;
@group(0) @binding(7) var shadow_out: texture_storage_2d<r32float, write>; // half-res

const SHADOW_BIAS: f32 = 0.03;

@compute @workgroup_size(8, 8)
fn shadow_lores_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let lo = textureDimensions(shadow_out);
    if (gid.x >= lo.x || gid.y >= lo.y) {
        return;
    }
    // The source full-res pixel this lo-res texel represents.
    let full = textureDimensions(gbuf_depth);
    let fc = clamp(
        vec2<i32>(i32(gid.x) * 2, i32(gid.y) * 2),
        vec2<i32>(0),
        vec2<i32>(i32(full.x) - 1, i32(full.y) - 1),
    );
    let depth = textureLoad(gbuf_depth, fc, 0).x;
    var shadow = 1.0; // 1 = lit

    if (depth > 0.0) {
        let nrm = textureLoad(gbuf_normal, fc, 0).xyz * 2.0 - 1.0;
        let sun = normalize(sun_dir.xyz);
        if (dot(nrm, sun) > 0.0) {
            let w = f32(full.x);
            let h = f32(full.y);
            let ndc_x = ((f32(fc.x) + 0.5) / w * 2.0 - 1.0) * camera.tan * camera.aspect;
            let ndc_y = (1.0 - (f32(fc.y) + 0.5) / h * 2.0) * camera.tan;
            let world = camera.eye
                + depth * ndc_x * camera.right
                + depth * ndc_y * camera.up
                + depth * camera.forward;
            let origin = world + nrm * SHADOW_BIAS;
            if (traverse_ray(origin, sun, camera.n, camera.dims.z).hit == 1u) {
                shadow = 0.0;
            }
        }
        // Back-facing (dot<=0): leave lit — matches the G-buffer; ndl=0 in shading.
    }

    textureStore(shadow_out, vec2<u32>(gid.x, gid.y), vec4<f32>(shadow, 0.0, 0.0, 0.0));
}
