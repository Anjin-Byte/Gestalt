// Composite pass: read the G-buffer (depth + world normal) plus the denoised
// GTAO term and shade — a directional sun (N·L, gated by the ray-traced shadow
// in normal.a) plus hemisphere ambient dimmed by AO. Writes the pre-TAA HDR
// colour buffer, not the screen. Standalone module (no traversal); concatenated
// after `sky.wgsl`, whose `sky_env`@10 uniform + `sky_color` paint the themed
// gradient on miss pixels (depth == 0) — the same sky the forward path drew, so
// `set_sky` theming keeps working under the deferred pipeline.

struct Camera {
    eye: vec3<f32>,
    tan: f32,
    forward: vec3<f32>,
    aspect: f32,
    right: vec3<f32>,
    n: f32,
    up: vec3<f32>,
    _pad: f32,
    dims: vec4<u32>, // width, height, k, flag bits (bit2 palette, bit7 truecolor)
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var gbuf_depth: texture_2d<f32>;
@group(0) @binding(2) var gbuf_normal: texture_2d<f32>;
@group(0) @binding(3) var gtao_ao: texture_2d<f32>;
@group(0) @binding(4) var output: texture_storage_2d<rgba16float, write>; // pre-TAA HDR colour
// Sun direction (xyz; w pad) — shared with gbuffer.wgsl so the direct term is
// lit by the same sun the shadow ray traced toward.
@group(0) @binding(5) var<uniform> sun_dir: vec4<f32>;
// Per-voxel albedo from the G-buffer (dims.w bit7 truecolor / bit2 palette).
// When set, the shaded albedo is this (sRGB→linear decoded) instead of the
// position-hash tint.
@group(0) @binding(6) var gbuf_albedo: texture_2d<f32>;

const SUN_COLOR: vec3<f32> = vec3<f32>(1.0, 0.96, 0.88);
const SKY_COLOR: vec3<f32> = vec3<f32>(0.42, 0.47, 0.58);
const GROUND_COLOR: vec3<f32> = vec3<f32>(0.16, 0.15, 0.13);

@compute @workgroup_size(8, 8)
fn composite_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let width = camera.dims.x;
    let height = camera.dims.y;
    if (gid.x >= width || gid.y >= height) {
        return;
    }
    let coord = vec2<i32>(i32(gid.x), i32(gid.y));
    let depth = textureLoad(gbuf_depth, coord, 0).x;

    var color: vec3<f32>;
    if (depth > 0.0) {
        let ntex = textureLoad(gbuf_normal, coord, 0);
        let normal = ntex.xyz * 2.0 - 1.0;
        // Sun visibility [0,1]: the G-buffer's inline ray-traced hard shadow
        // (1.0 whenever shadows are off — the write default).
        let shadow = ntex.a;
        let sun = normalize(sun_dir.xyz);
        let ndl = max(dot(normal, sun), 0.0);
        let up = clamp(normal.y * 0.5 + 0.5, 0.0, 1.0);
        let ao = textureLoad(gtao_ao, coord, 0).x; // GTAO visibility [0,1]
        let ambient = mix(GROUND_COLOR, SKY_COLOR, up) * ao;

        // Albedo source: per-voxel colour from the G-buffer when dims.w bit7
        // (truecolor) or bit2 (palette) is set — sRGB→linear decoded, since the
        // deferred chain shades in linear HDR. Otherwise the position-hash tint,
        // the prior look for occupancy-only scenes.
        var albedo: vec3<f32>;
        if ((camera.dims.w & (128u | 4u)) != 0u) {
            let srgb = textureLoad(gbuf_albedo, coord, 0).rgb;
            albedo = pow(srgb, vec3<f32>(2.2));
        } else {
            // Reconstruct the world hit position from eye-space depth + camera,
            // normalize to the grid, and tint. A white base keeps it from going
            // black at the origin corner.
            let w = f32(width);
            let h = f32(height);
            let ndc_x = ((f32(gid.x) + 0.5) / w * 2.0 - 1.0) * camera.tan * camera.aspect;
            let ndc_y = (1.0 - (f32(gid.y) + 0.5) / h * 2.0) * camera.tan;
            let vpos = vec3<f32>(depth * ndc_x, depth * ndc_y, depth);
            let world = camera.eye + vpos.x * camera.right + vpos.y * camera.up + vpos.z * camera.forward;
            let p = clamp(world / camera.n, vec3<f32>(0.0), vec3<f32>(1.0));
            albedo = mix(vec3<f32>(0.85), p, 0.7);
        }

        // AO dims ambient; the ray-traced shadow gates direct sun.
        color = albedo * (ambient + ndl * SUN_COLOR * shadow);
    } else {
        // The themed sky gradient (sky.wgsl, uniform @10) — dithered, same
        // bytes the forward path wrote, passed through TAA untouched.
        color = sky_color(vec2<u32>(gid.x, gid.y), f32(height)).rgb;
    }
    textureStore(output, vec2<u32>(gid.x, gid.y), vec4<f32>(color, 1.0));
}
