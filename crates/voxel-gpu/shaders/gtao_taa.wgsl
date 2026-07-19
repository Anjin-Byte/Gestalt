// Temporal anti-aliasing / accumulation — a WGSL port of Intel's reference TAA
// (XeGTAO/Source/Rendering/Shaders/IntelTAA.hlsli). Runs on the composited
// colour: reproject last frame, clip the history to the current colour
// neighbourhood (YCoCg variance clipping — the ghost-rejecting core), and blend
// by motion + depth confidence with a self-converging history weight. Because
// the GTAO pass rotates its sample noise per frame, accumulating here turns the
// cheap noisy AO into a clean, stable image. Static scene + moving camera, so
// reprojection is derived from depth + the previous camera (no velocity buffer).

struct Camera {
    eye: vec3<f32>, tan: f32,
    forward: vec3<f32>, aspect: f32,
    right: vec3<f32>, n: f32,
    up: vec3<f32>, _pad: f32,
    dims: vec4<u32>,
}
struct TaaParams {
    width: u32,
    height: u32,
    frame_index: u32,
    history_valid: u32, // 0 on the first frame / after resize → no reprojection
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> prev_camera: Camera;
@group(0) @binding(2) var color_cur: texture_2d<f32>;
@group(0) @binding(3) var color_hist: texture_2d<f32>; // prev resolved (rgb + confidence a)
@group(0) @binding(4) var gbuf_depth: texture_2d<f32>;
@group(0) @binding(5) var prev_depth: texture_2d<f32>;
@group(0) @binding(6) var out_color: texture_storage_2d<rgba8unorm, write>;   // → screen
@group(0) @binding(7) var out_history: texture_storage_2d<rgba16float, write>; // → next frame
@group(0) @binding(8) var<uniform> params: TaaParams;

// Motion (px) beyond which history is fully rejected; scaled to the viewport
// (IntelTAA uses 256 @ 1080p).
const VEL_REJECT_FRAC: f32 = 256.0 / 1080.0;
const GAMMA_MIN: f32 = 1.0;
const GAMMA_MAX: f32 = 2.0;

fn rgb2ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(c, vec3<f32>(0.25, 0.5, 0.25)),
        dot(c, vec3<f32>(0.5, 0.0, -0.5)),
        dot(c, vec3<f32>(-0.25, 0.5, -0.25)),
    );
}
fn ycocg2rgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(c, vec3<f32>(1.0, 1.0, -1.0)),
        dot(c, vec3<f32>(1.0, 0.0, 1.0)),
        dot(c, vec3<f32>(1.0, -1.0, -1.0)),
    );
}

// IntelTAA ClipToAABB: clip the history colour toward the current colour until
// it enters the neighbourhood box (centre ± extents), in YCoCg.
fn clip_to_aabb(hist: vec3<f32>, cur: vec3<f32>, centre: vec3<f32>, extents: vec3<f32>) -> vec3<f32> {
    let dir = cur - hist;
    let safe = sign(dir) * max(abs(dir), vec3<f32>(1e-6));
    let isect = ((centre - sign(dir) * extents) - hist) / safe;
    let t_pos = select(vec3<f32>(1e9), isect, isect >= vec3<f32>(0.0));
    let t = min(1.0, min(t_pos.x, min(t_pos.y, t_pos.z)));
    return select(hist, hist + dir * t, t < 1.0);
}

fn view_pos(screen_norm: vec2<f32>, vz: f32) -> vec3<f32> {
    let ndc_x = screen_norm.x * 2.0 - 1.0;
    let ndc_y = 1.0 - screen_norm.y * 2.0;
    return vec3<f32>(vz * ndc_x * camera.tan * camera.aspect, vz * ndc_y * camera.tan, vz);
}

fn cur_color(px: vec2<i32>, hi: vec2<i32>) -> vec3<f32> {
    return textureLoad(color_cur, clamp(px, vec2<i32>(0), hi), 0).xyz;
}

// Bilinear sample of the (rgb+a) history at a fractional pixel coord.
fn hist_bilinear(p: vec2<f32>, hi: vec2<i32>) -> vec4<f32> {
    let f = p - 0.5;
    let i0 = vec2<i32>(floor(f));
    let fr = f - floor(f);
    let c00 = textureLoad(color_hist, clamp(i0, vec2<i32>(0), hi), 0);
    let c10 = textureLoad(color_hist, clamp(i0 + vec2<i32>(1, 0), vec2<i32>(0), hi), 0);
    let c01 = textureLoad(color_hist, clamp(i0 + vec2<i32>(0, 1), vec2<i32>(0), hi), 0);
    let c11 = textureLoad(color_hist, clamp(i0 + vec2<i32>(1, 1), vec2<i32>(0), hi), 0);
    return mix(mix(c00, c10, fr.x), mix(c01, c11, fr.x), fr.y);
}

@compute @workgroup_size(8, 8)
fn taa_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    let px = vec2<i32>(i32(gid.x), i32(gid.y));
    let hi = vec2<i32>(i32(params.width) - 1, i32(params.height) - 1);
    let dims = vec2<f32>(f32(params.width), f32(params.height));
    let current = textureLoad(color_cur, px, 0).xyz;

    // Default: no valid history → output current, seed confidence 0.5.
    var out_rgb = current;
    var new_a = 0.5;

    let depth = textureLoad(gbuf_depth, px, 0).x;
    let screen_norm = (vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5) / dims;

    // Reproject (only meaningful for surface pixels with valid prior history).
    if (params.history_valid == 1u && depth > 0.0) {
        let vpos = view_pos(screen_norm, depth);
        let world = camera.eye + vpos.x * camera.right + vpos.y * camera.up + vpos.z * camera.forward;
        let rel = world - prev_camera.eye;
        let pv = vec3<f32>(dot(rel, prev_camera.right), dot(rel, prev_camera.up), dot(rel, prev_camera.forward));
        if (pv.z > 1e-4) {
            let pndc_x = pv.x / (pv.z * prev_camera.tan * prev_camera.aspect);
            let pndc_y = pv.y / (pv.z * prev_camera.tan);
            let prev_screen = vec2<f32>(pndc_x * 0.5 + 0.5, 0.5 - pndc_y * 0.5);
            if (all(prev_screen >= vec2<f32>(0.0)) && all(prev_screen < vec2<f32>(1.0))) {
                let vel_px = length((prev_screen - screen_norm) * dims);
                let vel_conf = clamp(1.0 - vel_px / (dims.y * VEL_REJECT_FRAC), 0.0, 1.0);

                // Depth confidence: reprojected depth vs stored prev depth (soft).
                let pdpx = clamp(vec2<i32>(prev_screen * dims), vec2<i32>(0), hi);
                let pd = textureLoad(prev_depth, pdpx, 0).x;
                let d_err = abs(pd - pv.z) / max(pv.z, 1e-3);
                let depth_conf = 1.0 - smoothstep(0.02, 0.06, d_err);

                if (vel_conf * depth_conf > 0.0) {
                    let hist = hist_bilinear(prev_screen * dims, hi);
                    // Current 3×3 neighbourhood mean/variance in YCoCg.
                    let gamma = mix(GAMMA_MIN, GAMMA_MAX, vel_conf * vel_conf);
                    var m1 = vec3<f32>(0.0);
                    var m2 = vec3<f32>(0.0);
                    for (var dy = -1; dy <= 1; dy = dy + 1) {
                        for (var dx = -1; dx <= 1; dx = dx + 1) {
                            let c = rgb2ycocg(cur_color(px + vec2<i32>(dx, dy), hi));
                            m1 += c;
                            m2 += c * c;
                        }
                    }
                    let mean = m1 / 9.0;
                    let variance = sqrt(max(vec3<f32>(1e-5), m2 / 9.0 - mean * mean)) * gamma;
                    let clipped = ycocg2rgb(clip_to_aabb(rgb2ycocg(hist.xyz), rgb2ycocg(current), mean, variance));

                    let weight = hist.w * vel_conf * depth_conf;
                    out_rgb = mix(current, clipped, clamp(weight, 0.0, 1.0));
                    new_a = clamp(1.0 / (2.0 - weight), 0.0, 1.0);
                }
            }
        }
    }

    // The hover-cursor ring (cursor.wgsl @9, concatenated ahead of this module)
    // tints the SCREEN store only — history stays clean so a moving ring never
    // ghosts into the accumulation. The hit voxel is reconstructed from depth by
    // stepping an epsilon along the ray past the entry face (the forward path had
    // the exact hit voxel; at extreme corner-grazing the epsilon can land one
    // voxel off, invisible for a highlight band). Inactive cursor (enabled == 0,
    // the zeroed-uniform default) leaves the store byte-identical.
    var screen_rgb = out_rgb;
    if (cursor.enabled > 0.5 && depth > 0.0) {
        let vpos = view_pos(screen_norm, depth);
        let world = camera.eye + vpos.x * camera.right + vpos.y * camera.up + vpos.z * camera.forward;
        let ray = normalize(world - camera.eye);
        let inside = clamp(world + ray * 1e-3, vec3<f32>(0.0), vec3<f32>(camera.n - 1.0));
        screen_rgb = cursor_tint(out_rgb, vec3<u32>(floor(inside)));
    }
    textureStore(out_color, vec2<u32>(gid.x, gid.y), vec4<f32>(screen_rgb, 1.0));
    textureStore(out_history, vec2<u32>(gid.x, gid.y), vec4<f32>(out_rgb, new_a));
}
