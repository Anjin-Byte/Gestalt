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
    dims: vec4<u32>, // width, height, k, flags (bit2 palette, bit3 AO off, bit5 half-res shadow, bit7 truecolor)
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
// The ½×½ sun-shadow from the lores pass (dims.w bit5 — the "low" shadow
// tier). Read with the joint-bilateral upsample below; ignored otherwise.
@group(0) @binding(7) var shadow_lores: texture_2d<f32>;

const SUN_COLOR: vec3<f32> = vec3<f32>(1.0, 0.96, 0.88);
const SKY_COLOR: vec3<f32> = vec3<f32>(0.42, 0.47, 0.58);
const GROUND_COLOR: vec3<f32> = vec3<f32>(0.16, 0.15, 0.13);

// Joint-bilateral upsample of the half-res shadow: lo-res taps are gated by
// depth AND normal similarity to this full-res pixel. The depth gate stops
// silhouette halos; the normal gate stops the bright edge fringe where a lit
// face abuts a shadowed face at ~equal depth (a voxel edge) — without it the
// depth gate can't tell the two faces apart and bleeds light in.
const SHADOW_UPSAMPLE_DEPTH_REL: f32 = 0.04;
const SHADOW_UPSAMPLE_NORMAL_MIN: f32 = 0.5; // reject neighbours >60° apart (diff face)

// Anisotropic upsample: on a grazing surface (large screen-space depth
// gradient) the ½-res shadow aliases (moiré); widen the gather ALONG the
// gradient (the grazing direction) so more lo-res samples average →
// band-limited. Off the grazing axis it stays ~bilinear, so head-on shadows
// aren't over-blurred.
const SHADOW_ANISO_TAPS: i32 = 4;       // 2·this+1 taps along the grazing axis
const SHADOW_ANISO_SCALE: f32 = 300.0;  // grazing-ness → footprint length (tunable)
const SHADOW_ANISO_MAX: f32 = 10.0;     // max footprint (full-res px)

// Whether lo-res texel `q` is the same surface as the full-res pixel — the
// joint gate shared by both upsamplers. `src` is the full-res pixel the lo-res
// texel was actually traced at (2·q).
fn lores_same_surface(q: vec2<i32>, fullhi: vec2<i32>, depth: f32, normal: vec3<f32>) -> bool {
    let src = clamp(q * 2, vec2<i32>(0), fullhi);
    let qz = textureLoad(gbuf_depth, src, 0).x;
    let qn = textureLoad(gbuf_normal, src, 0).xyz * 2.0 - 1.0;
    return qz > 0.0
        && abs(qz - depth) <= depth * SHADOW_UPSAMPLE_DEPTH_REL
        && dot(qn, normal) >= SHADOW_UPSAMPLE_NORMAL_MIN;
}

// Bilinear joint-bilateral upsample: the 4 nearest lo-res texels,
// bilinear-weighted, each gated to the same surface.
fn shadow_bilinear(coord: vec2<i32>, fullhi: vec2<i32>, depth: f32, normal: vec3<f32>) -> f32 {
    let lo = textureDimensions(shadow_lores);
    let hi = vec2<i32>(i32(lo.x) - 1, i32(lo.y) - 1);
    let lf = vec2<f32>(f32(coord.x), f32(coord.y)) * 0.5;
    let l0 = vec2<i32>(i32(floor(lf.x)), i32(floor(lf.y)));
    let fr = lf - vec2<f32>(f32(l0.x), f32(l0.y));
    var ssum = 0.0;
    var wsum = 0.0;
    for (var j = 0; j <= 1; j = j + 1) {
        for (var i = 0; i <= 1; i = i + 1) {
            let q = clamp(l0 + vec2<i32>(i, j), vec2<i32>(0), hi);
            let bw = select(1.0 - fr.x, fr.x, i == 1) * select(1.0 - fr.y, fr.y, j == 1);
            if (lores_same_surface(q, fullhi, depth, normal)) {
                ssum = ssum + bw * textureLoad(shadow_lores, q, 0).x;
                wsum = wsum + bw;
            }
        }
    }
    if (wsum > 0.0) {
        return ssum / wsum;
    }
    let nn = clamp(vec2<i32>(i32(round(lf.x)), i32(round(lf.y))), vec2<i32>(0), hi);
    return textureLoad(shadow_lores, nn, 0).x;
}

// Anisotropic joint-bilateral upsample: gather taps along the screen
// depth-gradient (grazing) direction, length scaled by grazing-ness, each gated.
fn shadow_aniso(coord: vec2<i32>, fullhi: vec2<i32>, depth: f32, normal: vec3<f32>) -> f32 {
    let lo = textureDimensions(shadow_lores);
    let hi = vec2<i32>(i32(lo.x) - 1, i32(lo.y) - 1);
    let dxp = textureLoad(gbuf_depth, clamp(coord + vec2<i32>(1, 0), vec2<i32>(0), fullhi), 0).x;
    let dxm = textureLoad(gbuf_depth, clamp(coord - vec2<i32>(1, 0), vec2<i32>(0), fullhi), 0).x;
    let dyp = textureLoad(gbuf_depth, clamp(coord + vec2<i32>(0, 1), vec2<i32>(0), fullhi), 0).x;
    let dym = textureLoad(gbuf_depth, clamp(coord - vec2<i32>(0, 1), vec2<i32>(0), fullhi), 0).x;
    let grad = vec2<f32>(dxp - dxm, dyp - dym) * 0.5;
    let gmag = length(grad);
    let span = clamp(gmag / max(depth, 1.0) * SHADOW_ANISO_SCALE, 1.0, SHADOW_ANISO_MAX);
    // Head-on (little depth gradient) → isotropic bilinear; only stretch the
    // kernel on grazing surfaces, where the ½-res shadow actually aliases.
    if (span < 1.5) {
        return shadow_bilinear(coord, fullhi, depth, normal);
    }
    let dir = select(vec2<f32>(1.0, 0.0), grad / max(gmag, 1e-6), gmag > 1.0e-4);
    var ssum = 0.0;
    var wsum = 0.0;
    for (var i = -SHADOW_ANISO_TAPS; i <= SHADOW_ANISO_TAPS; i = i + 1) {
        let fpos = vec2<f32>(f32(coord.x), f32(coord.y)) + dir * (f32(i) * span);
        let q = clamp(
            vec2<i32>(i32(floor(fpos.x * 0.5)), i32(floor(fpos.y * 0.5))),
            vec2<i32>(0),
            hi,
        );
        if (lores_same_surface(q, fullhi, depth, normal)) {
            let fi = f32(i) / f32(SHADOW_ANISO_TAPS + 1);
            let wgt = exp(-2.0 * fi * fi);
            ssum = ssum + wgt * textureLoad(shadow_lores, q, 0).x;
            wsum = wsum + wgt;
        }
    }
    if (wsum > 0.0) {
        return ssum / wsum;
    }
    return textureLoad(shadow_lores, clamp(coord / 2, vec2<i32>(0), hi), 0).x;
}

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
        // Sun visibility [0,1]. Half-res tier (dims.w bit5): anisotropic
        // joint-bilateral upsample of the ½×½ lores trace. Else: the G-buffer's
        // inline full-res trace in normal.a (1.0 whenever shadows are off —
        // the write default).
        var shadow = ntex.a;
        if ((camera.dims.w & 32u) != 0u) {
            let fullhi = vec2<i32>(i32(width) - 1, i32(height) - 1);
            let normal_for_gate = ntex.xyz * 2.0 - 1.0;
            shadow = shadow_aniso(coord, fullhi, depth, normal_for_gate);
        }
        let sun = normalize(sun_dir.xyz);
        let ndl = max(dot(normal, sun), 0.0);
        let up = clamp(normal.y * 0.5 + 0.5, 0.0, 1.0);
        // GTAO visibility [0,1]; dims.w bit3 = AO disabled (the GTAO/denoise
        // dispatches were skipped, the texture is stale) → full visibility.
        var ao = 1.0;
        if ((camera.dims.w & 8u) == 0u) {
            ao = textureLoad(gtao_ao, coord, 0).x;
        }
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
        // The themed sky gradient (sky.wgsl, uniform @10), decoded to linear —
        // the whole pre-TAA chain shades in linear light, and the TAA's final
        // sRGB encode round-trips these back to the exact bytes the forward
        // path wrote (dither survives the round trip).
        color = pow(sky_color(vec2<u32>(gid.x, gid.y), f32(height)).rgb, vec3<f32>(2.2));
    }
    textureStore(output, vec2<u32>(gid.x, gid.y), vec4<f32>(color, 1.0));
}
