// GTAO main pass — a WGSL port of Intel's XeGTAO `XeGTAO_MainPass`
// (Source/Rendering/Shaders/XeGTAO.hlsli), implementing the ground-truth AO of
// Jimenez et al. 2016 (the bundled paper). Per pixel: reconstruct viewspace
// position from the G-buffer depth, rotate the world normal to viewspace, then
// over `SLICE_COUNT` slices × `STEPS_PER_SLICE` steps search the screen-space
// horizon (with a distance falloff + thin-occluder term) and integrate the
// cosine-weighted arc (paper Eq. 7) into a visibility term.
//
// Inputs: eye-space linear depth (R32Float) + world face normal (RGBA8Unorm).
// Output: AO visibility in [0,1] (R32Float). Denoise + temporal come next; raw
// here, so quality presets (slice×step) read noisy per-frame.

struct Camera {
    eye: vec3<f32>,
    tan: f32,
    forward: vec3<f32>,
    aspect: f32,
    right: vec3<f32>,
    n: f32,
    up: vec3<f32>,
    _pad: f32,
    dims: vec4<u32>, // width, height, k, _
}

// Runtime GTAO quality/tuning: slice & step counts (the quality preset) and the
// effect radius in voxels. Lets the host sweep Low/Med/High/Ultra without
// recompiling.
struct GtaoParams {
    slice_count: f32,
    steps_per_slice: f32,
    effect_radius: f32,
    frame_index: u32, // rotates the sample noise per frame so the TAA can average it
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var gbuf_depth: texture_2d<f32>;
@group(0) @binding(2) var gbuf_normal: texture_2d<f32>;
@group(0) @binding(3) var ao_out: texture_storage_2d<r32float, write>;
@group(0) @binding(4) var<uniform> params: GtaoParams;
@group(0) @binding(5) var edges_out: texture_storage_2d<r32float, write>; // packed byte in [0,1]

// Depth-discontinuity edges (XeGTAO_CalculateEdges/PackEdges): per-pixel L/R/T/B
// edge strengths in [0,1] (1 = flat, blur freely; 0 = hard edge, don't blur
// across), quantized 2 bits each and packed into one R8 byte for the denoise.
fn pack_edges(center_z: f32, l: f32, r: f32, t: f32, b: f32) -> f32 {
    let e = vec4<f32>(l, r, t, b) - center_z;
    let slope_lr = (e.y - e.x) * 0.5;
    let slope_tb = (e.w - e.z) * 0.5;
    let adj = e + vec4<f32>(slope_lr, -slope_lr, slope_tb, -slope_tb);
    let m = min(abs(e), abs(adj));
    let edges = clamp(vec4<f32>(1.25) - m / (center_z * 0.011), vec4<f32>(0.0), vec4<f32>(1.0));
    let q = round(edges * 2.9);
    return dot(q, vec4<f32>(64.0, 16.0, 4.0, 1.0)) / 255.0;
}

const PI: f32 = 3.1415927;
const HALF_PI: f32 = 1.5707963;

// Fixed XeGTAO defaults (the quality-independent tuning).
const RADIUS_MULTIPLIER: f32 = 1.457;
const FALLOFF_RANGE: f32 = 0.615;
const SAMPLE_DISTRIBUTION_POWER: f32 = 2.0;
const THIN_OCCLUDER_COMPENSATION: f32 = 0.0;
const FINAL_VALUE_POWER: f32 = 2.2;
const PIXEL_TOO_CLOSE: f32 = 1.3;

// XeGTAO's fast acos (Eberly); |error| < 0.0007.
fn fast_acos(in_x: f32) -> f32 {
    let x = abs(in_x);
    var res = -0.156583 * x + HALF_PI;
    res *= sqrt(1.0 - x);
    if (in_x >= 0.0) { return res; }
    return PI - res;
}

// Viewspace position of a normalized screen pos at eye-space depth `vz`.
// Our viewspace: x=right, y=up, z=forward (depth, positive in front).
fn view_pos(screen_norm: vec2<f32>, vz: f32) -> vec3<f32> {
    let ndc_x = screen_norm.x * 2.0 - 1.0;
    let ndc_y = 1.0 - screen_norm.y * 2.0;
    return vec3<f32>(vz * ndc_x * camera.tan * camera.aspect, vz * ndc_y * camera.tan, vz);
}

// Eye-space depth at an integer pixel; a sky/miss sentinel (<=0) becomes "far"
// so it contributes no occlusion.
fn depth_at(px: vec2<i32>) -> f32 {
    let d = textureLoad(gbuf_depth, px, 0).x;
    if (d <= 0.0) { return 1e9; }
    return d;
}

// Cheap per-pixel 2D noise (slice rotation, step jitter). Replaced by a
// spatiotemporal sequence when temporal accumulation lands.
fn noise2(px: vec2<u32>) -> vec2<f32> {
    var h = px.x * 374761393u + px.y * 668265263u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    let a = f32(h & 0xffffu) / 65536.0;
    let b = f32((h >> 16u) & 0xffffu) / 65536.0;
    return vec2<f32>(a, b);
}

@compute @workgroup_size(8, 8)
fn gtao_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let width = camera.dims.x;
    let height = camera.dims.y;
    if (gid.x >= width || gid.y >= height) {
        return;
    }
    let px = vec2<i32>(i32(gid.x), i32(gid.y));
    let viewspaceZ = textureLoad(gbuf_depth, px, 0).x;
    if (viewspaceZ <= 0.0) {
        textureStore(ao_out, vec2<u32>(gid.x, gid.y), vec4<f32>(1.0)); // sky: unoccluded
        textureStore(edges_out, vec2<u32>(gid.x, gid.y), vec4<f32>(0.0)); // hard edges around sky
        return;
    }
    // Depth-edge weights for the denoise (cardinal neighbors; sky→far→hard edge).
    let edge_l = depth_at(vec2<i32>(max(px.x - 1, 0), px.y));
    let edge_r = depth_at(vec2<i32>(min(px.x + 1, i32(width) - 1), px.y));
    let edge_t = depth_at(vec2<i32>(px.x, max(px.y - 1, 0)));
    let edge_b = depth_at(vec2<i32>(px.x, min(px.y + 1, i32(height) - 1)));
    textureStore(
        edges_out,
        vec2<u32>(gid.x, gid.y),
        vec4<f32>(pack_edges(viewspaceZ, edge_l, edge_r, edge_t, edge_b)),
    );

    let w = f32(width);
    let h = f32(height);
    let pixel_size = vec2<f32>(1.0 / w, 1.0 / h);
    let screen_norm = (vec2<f32>(f32(gid.x), f32(gid.y)) + 0.5) * pixel_size;

    let world_n = textureLoad(gbuf_normal, px, 0).xyz * 2.0 - 1.0;
    let viewspaceNormal = vec3<f32>(
        dot(world_n, camera.right),
        dot(world_n, camera.up),
        dot(world_n, camera.forward),
    );

    let pixCenterPos = view_pos(screen_norm, viewspaceZ);
    let viewVec = normalize(-pixCenterPos);

    let slice_count = params.slice_count;
    let steps_per_slice = params.steps_per_slice;
    let effectRadius = params.effect_radius * RADIUS_MULTIPLIER;
    let falloffRange = FALLOFF_RANGE * effectRadius;
    let falloffFrom = effectRadius * (1.0 - FALLOFF_RANGE);
    let falloffMul = -1.0 / falloffRange;
    let falloffAdd = falloffFrom / falloffRange + 1.0;

    // Per-pixel base noise, rotated per frame along the R2 low-discrepancy
    // sequence so successive frames decorrelate and the TAA averages to clean.
    let base = noise2(gid.xy);
    let fi = f32(params.frame_index);
    let noiseSlice = fract(base.x + fi * 0.7548776662);
    let noiseSample = fract(base.y + fi * 0.5698402910);

    // viewspace size of one pixel (x) at this depth → screen-space radius (px).
    let pixelVSSizeX = viewspaceZ * camera.tan * camera.aspect * 2.0 * pixel_size.x;
    let screenspaceRadius = effectRadius / pixelVSSizeX;
    let minS = PIXEL_TOO_CLOSE / screenspaceRadius;

    var visibility = 0.0;
    visibility += clamp((10.0 - screenspaceRadius) / 100.0, 0.0, 1.0) * 0.5; // small-radius fade

    for (var slice = 0.0; slice < slice_count; slice += 1.0) {
        let sliceK = (slice + noiseSlice) / slice_count;
        let phi = sliceK * PI;
        let cosPhi = cos(phi);
        let sinPhi = sin(phi);
        var omega = vec2<f32>(cosPhi, -sinPhi) * screenspaceRadius; // screen px

        let directionVec = vec3<f32>(cosPhi, sinPhi, 0.0);
        let orthoDirectionVec = directionVec - dot(directionVec, viewVec) * viewVec;
        let axisVec = normalize(cross(orthoDirectionVec, viewVec));
        let projectedNormalVec = viewspaceNormal - axisVec * dot(viewspaceNormal, axisVec);
        let signNorm = sign(dot(orthoDirectionVec, projectedNormalVec));
        var projectedNormalVecLength = length(projectedNormalVec);
        let cosNorm = clamp(dot(projectedNormalVec, viewVec) / projectedNormalVecLength, 0.0, 1.0);
        let nAngle = signNorm * fast_acos(cosNorm);

        let lowHorizonCos0 = cos(nAngle + HALF_PI);
        let lowHorizonCos1 = cos(nAngle - HALF_PI);
        var horizonCos0 = lowHorizonCos0;
        var horizonCos1 = lowHorizonCos1;

        for (var step = 0.0; step < steps_per_slice; step += 1.0) {
            let stepBaseNoise = (slice + step * steps_per_slice) * 0.6180339887498948;
            let stepNoise = fract(noiseSample + stepBaseNoise);
            var s = (step + stepNoise) / steps_per_slice;
            s = pow(s, SAMPLE_DISTRIBUTION_POWER);
            s += minS;

            let sampleOffsetPx = round(s * omega);
            let sampleOffset = sampleOffsetPx * pixel_size;

            let sp0 = screen_norm + sampleOffset;
            let sp1 = screen_norm - sampleOffset;
            let sz0 = depth_at(vec2<i32>(clamp(round(sp0 * vec2<f32>(w, h)), vec2<f32>(0.0), vec2<f32>(w - 1.0, h - 1.0))));
            let sz1 = depth_at(vec2<i32>(clamp(round(sp1 * vec2<f32>(w, h)), vec2<f32>(0.0), vec2<f32>(w - 1.0, h - 1.0))));
            let samplePos0 = view_pos(sp0, sz0);
            let samplePos1 = view_pos(sp1, sz1);

            let sampleDelta0 = samplePos0 - pixCenterPos;
            let sampleDelta1 = samplePos1 - pixCenterPos;
            let sampleDist0 = length(sampleDelta0);
            let sampleDist1 = length(sampleDelta1);
            let sampleHorizonVec0 = sampleDelta0 / sampleDist0;
            let sampleHorizonVec1 = sampleDelta1 / sampleDist1;

            // thin-occluder-aware distance falloff (z scaled by 1+comp)
            let falloffBase0 = length(vec3<f32>(sampleDelta0.x, sampleDelta0.y, sampleDelta0.z * (1.0 + THIN_OCCLUDER_COMPENSATION)));
            let falloffBase1 = length(vec3<f32>(sampleDelta1.x, sampleDelta1.y, sampleDelta1.z * (1.0 + THIN_OCCLUDER_COMPENSATION)));
            let weight0 = clamp(falloffBase0 * falloffMul + falloffAdd, 0.0, 1.0);
            let weight1 = clamp(falloffBase1 * falloffMul + falloffAdd, 0.0, 1.0);

            var shc0 = dot(sampleHorizonVec0, viewVec);
            var shc1 = dot(sampleHorizonVec1, viewVec);
            shc0 = mix(lowHorizonCos0, shc0, weight0);
            shc1 = mix(lowHorizonCos1, shc1, weight1);
            horizonCos0 = max(horizonCos0, shc0);
            horizonCos1 = max(horizonCos1, shc1);
        }

        projectedNormalVecLength = mix(projectedNormalVecLength, 1.0, 0.05); // overdarkening fudge

        let h0 = -fast_acos(horizonCos1);
        let h1 = fast_acos(horizonCos0);
        let iarc0 = (cosNorm + 2.0 * h0 * sin(nAngle) - cos(2.0 * h0 - nAngle)) / 4.0;
        let iarc1 = (cosNorm + 2.0 * h1 * sin(nAngle) - cos(2.0 * h1 - nAngle)) / 4.0;
        visibility += projectedNormalVecLength * (iarc0 + iarc1);
    }

    visibility = visibility / slice_count;
    visibility = pow(clamp(visibility, 0.0, 1.0), FINAL_VALUE_POWER);
    visibility = max(0.03, visibility);

    textureStore(ao_out, vec2<u32>(gid.x, gid.y), vec4<f32>(visibility));
}
