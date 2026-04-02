// R-3: Hi-Z Pyramid Build
//
// Two entry points:
//   build_mip0 — copies depth_texture (depth32float) → hiz mip 0 (r32float)
//   build_mip  — downsamples mip N-1 → mip N via max of 2x2 region
//
// See: docs/Resident Representation/stages/R-3-hiz-build.md

struct HizParams {
    src_width:  u32,
    src_height: u32,
    dst_width:  u32,
    dst_height: u32,
};

// ─── Mip 0: depth → hiz ────────────────────────────────────────────────

@group(0) @binding(0) var depth_src: texture_depth_2d;
@group(0) @binding(1) var hiz_dst:   texture_storage_2d<r32float, write>;
@group(0) @binding(2) var<uniform> params: HizParams;

@compute @workgroup_size(8, 8, 1)
fn build_mip0(@builtin(global_invocation_id) gid: vec3u) {
    let coords = vec2i(gid.xy);
    if coords.x >= i32(params.dst_width) || coords.y >= i32(params.dst_height) {
        return;
    }
    let depth = textureLoad(depth_src, coords, 0);
    textureStore(hiz_dst, coords, vec4f(depth, 0.0, 0.0, 0.0));
}

// ─── Mip 1..N: max-depth downsample ────────────────────────────────────

@group(0) @binding(0) var hiz_src: texture_2d<f32>;
@group(0) @binding(1) var hiz_ds_dst: texture_storage_2d<r32float, write>;
@group(0) @binding(2) var<uniform> ds_params: HizParams;

@compute @workgroup_size(8, 8, 1)
fn build_mip(@builtin(global_invocation_id) gid: vec3u) {
    let out = vec2i(gid.xy);
    if out.x >= i32(ds_params.dst_width) || out.y >= i32(ds_params.dst_height) {
        return;
    }

    let src = out * 2;
    let src_max = vec2i(i32(ds_params.src_width) - 1, i32(ds_params.src_height) - 1);

    // Clamp to handle odd-dimension mip levels (conservative: duplicates edge texel)
    let s00 = clamp(src + vec2i(0, 0), vec2i(0), src_max);
    let s10 = clamp(src + vec2i(1, 0), vec2i(0), src_max);
    let s01 = clamp(src + vec2i(0, 1), vec2i(0), src_max);
    let s11 = clamp(src + vec2i(1, 1), vec2i(0), src_max);

    let d00 = textureLoad(hiz_src, s00, 0).r;
    let d10 = textureLoad(hiz_src, s10, 0).r;
    let d01 = textureLoad(hiz_src, s01, 0).r;
    let d11 = textureLoad(hiz_src, s11, 0).r;

    let max_depth = max(max(d00, d10), max(d01, d11));
    textureStore(hiz_ds_dst, out, vec4f(max_depth, 0.0, 0.0, 0.0));
}
