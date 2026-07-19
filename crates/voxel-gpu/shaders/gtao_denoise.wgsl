// GTAO spatial denoise — algorithm-faithful WGSL port of XeGTAO_Denoise
// (XeGTAO.hlsli:734-826): an edge-aware 3×3 bilateral on the AO term, weighting
// each neighbor by the depth-discontinuity edges written by the main pass (so
// occlusion doesn't bleed across silhouettes). Run twice (sharp pass → soft
// pass) via the `final_apply` flag. One pixel per thread + textureLoad (not
// XeGTAO's 2px/thread + Gather + uint packing — that's a bandwidth optimization
// we don't need; this is the same filter, plainly).

@group(0) @binding(0) var src_ao: texture_2d<f32>;
@group(0) @binding(1) var src_edges: texture_2d<f32>;
@group(0) @binding(2) var dst_ao: texture_storage_2d<r32float, write>;
@group(0) @binding(3) var<uniform> params: DenoiseParams;

struct DenoiseParams {
    blur_beta: f32,
    final_apply: u32, // unused for now (beta already differs per pass); reserved
    width: u32,
    height: u32,
}

// Unpack the R8-style packed edges (L,R,T,B, 2 bits each) from the float store.
fn unpack_edges(p: f32) -> vec4<f32> {
    let e = u32(p * 255.0 + 0.5);
    return vec4<f32>(
        f32((e >> 6u) & 3u),
        f32((e >> 4u) & 3u),
        f32((e >> 2u) & 3u),
        f32(e & 3u),
    ) / 3.0;
}

fn ao_at(px: vec2<i32>, lo: vec2<i32>, hi: vec2<i32>) -> f32 {
    return textureLoad(src_ao, clamp(px, lo, hi), 0).x;
}
fn edges_at(px: vec2<i32>, lo: vec2<i32>, hi: vec2<i32>) -> vec4<f32> {
    return unpack_edges(textureLoad(src_edges, clamp(px, lo, hi), 0).x);
}

@compute @workgroup_size(8, 8)
fn denoise_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let lo = vec2<i32>(0, 0);
    let hi = vec2<i32>(i32(params.width) - 1, i32(params.height) - 1);

    // Center edges, with XeGTAO's cross-pixel symmetry enforcement + leak.
    var edges_c = edges_at(p, lo, hi);
    let edges_l = edges_at(p + vec2<i32>(-1, 0), lo, hi);
    let edges_r = edges_at(p + vec2<i32>(1, 0), lo, hi);
    let edges_t = edges_at(p + vec2<i32>(0, -1), lo, hi);
    let edges_b = edges_at(p + vec2<i32>(0, 1), lo, hi);
    edges_c *= vec4<f32>(edges_l.y, edges_r.x, edges_t.w, edges_b.z);
    // Allow a little leak when ≥3 edges are active (reduces aliasing).
    let edginess = clamp(4.0 - 2.5 - dot(edges_c, vec4<f32>(1.0)), 0.0, 1.0) / (4.0 - 2.5) * 0.5;
    edges_c = clamp(edges_c + edginess, vec4<f32>(0.0), vec4<f32>(1.0));

    let diag_w = 0.85 * 0.5;
    let w_tl = diag_w * (edges_c.x * edges_l.z + edges_c.z * edges_t.x);
    let w_tr = diag_w * (edges_c.z * edges_t.y + edges_c.y * edges_r.z);
    let w_bl = diag_w * (edges_c.w * edges_b.x + edges_c.x * edges_l.w);
    let w_br = diag_w * (edges_c.y * edges_r.w + edges_c.w * edges_b.y);

    var sum_w = params.blur_beta;
    var sum = ao_at(p, lo, hi) * sum_w;
    // cardinals
    sum += ao_at(p + vec2<i32>(-1, 0), lo, hi) * edges_c.x; sum_w += edges_c.x;
    sum += ao_at(p + vec2<i32>(1, 0), lo, hi) * edges_c.y; sum_w += edges_c.y;
    sum += ao_at(p + vec2<i32>(0, -1), lo, hi) * edges_c.z; sum_w += edges_c.z;
    sum += ao_at(p + vec2<i32>(0, 1), lo, hi) * edges_c.w; sum_w += edges_c.w;
    // diagonals
    sum += ao_at(p + vec2<i32>(-1, -1), lo, hi) * w_tl; sum_w += w_tl;
    sum += ao_at(p + vec2<i32>(1, -1), lo, hi) * w_tr; sum_w += w_tr;
    sum += ao_at(p + vec2<i32>(-1, 1), lo, hi) * w_bl; sum_w += w_bl;
    sum += ao_at(p + vec2<i32>(1, 1), lo, hi) * w_br; sum_w += w_br;

    textureStore(dst_ao, vec2<u32>(gid.x, gid.y), vec4<f32>(sum / sum_w));
}
