// R-4: Hi-Z Occlusion Cull
//
// One thread per slot. Tests chunk AABB depth against Hi-Z pyramid.
// Writes visibility[slot] = 0 (occluded) or 1 (visible).
//
// Frustum culling is handled by the frustum pre-cull pass (frustum_cull.wgsl)
// which runs before the depth prepass. This shader only does depth occlusion.

struct Camera {
    view_proj: mat4x4f,
    position: vec4f,
};

@group(0) @binding(0) var<uniform>            camera:     Camera;
@group(0) @binding(1) var<storage, read>      aabb_buf:   array<vec4f>;
@group(0) @binding(2) var<storage, read>      flags_buf:  array<u32>;
@group(0) @binding(3) var                     hiz_pyramid: texture_2d<f32>;
@group(0) @binding(4) var<storage, read_write> visibility: array<u32>;
@group(0) @binding(5) var<uniform>            cull_params: vec4u; // x=slot_count, y=hiz_w, z=hiz_h, w=pass2_mode
@group(0) @binding(6) var<storage, read>      pass1_vis:   array<u32>;

const IS_EMPTY_BIT: u32 = 1u;

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let slot = gid.x;
    if slot >= cull_params.x { return; }

    // Pass 2 mode: skip Pass 1 survivors (already resolved)
    if cull_params.w == 1u && pass1_vis[slot] != 0u {
        // visibility[slot] already has pass1 result from the copy
        return;
    }

    // Empty or degenerate → cull
    if (flags_buf[slot] & IS_EMPTY_BIT) != 0u {
        visibility[slot] = 0u;
        return;
    }

    let aabb_min = aabb_buf[slot * 2u].xyz;
    let aabb_max = aabb_buf[slot * 2u + 1u].xyz;

    if aabb_min.x >= aabb_max.x || aabb_min.y >= aabb_max.y || aabb_min.z >= aabb_max.z {
        visibility[slot] = 0u;
        return;
    }

    // Project 8 corners to NDC
    var ndc_min = vec3f(1e10);
    var ndc_max = vec3f(-1e10);
    var min_z_ndc: f32 = 1e10;
    var corners_behind = 0u;

    for (var i = 0u; i < 8u; i++) {
        let corner = vec4f(
            select(aabb_min.x, aabb_max.x, (i & 1u) != 0u),
            select(aabb_min.y, aabb_max.y, (i & 2u) != 0u),
            select(aabb_min.z, aabb_max.z, (i & 4u) != 0u),
            1.0
        );
        let clip = camera.view_proj * corner;

        if clip.w <= 0.0 {
            corners_behind += 1u;
            continue;
        }

        let ndc = clip.xyz / clip.w;
        ndc_min = min(ndc_min, ndc);
        ndc_max = max(ndc_max, ndc);
        min_z_ndc = min(min_z_ndc, ndc.z);
    }

    // Fully behind camera → cull
    if corners_behind == 8u {
        visibility[slot] = 0u;
        return;
    }

    // Straddles near plane: check if valid corners project on-screen
    if corners_behind > 0u {
        if ndc_max.x < -1.0 || ndc_min.x > 1.0 ||
           ndc_max.y < -1.0 || ndc_min.y > 1.0 {
            visibility[slot] = 0u;
        } else {
            visibility[slot] = 1u;
        }
        return;
    }

    // Frustum cull: entirely outside viewport → cull
    if ndc_max.x < -1.0 || ndc_min.x > 1.0 ||
       ndc_max.y < -1.0 || ndc_min.y > 1.0 ||
       ndc_max.z < 0.0  || ndc_min.z > 1.0 {
        visibility[slot] = 0u;
        return;
    }

    // Partially off-screen — skip Hi-Z, conservatively visible
    if ndc_min.x < -1.0 || ndc_max.x > 1.0 ||
       ndc_min.y < -1.0 || ndc_max.y > 1.0 {
        visibility[slot] = 1u;
        return;
    }

    // Convert NDC → screen coords
    let hiz_w = f32(cull_params.y);
    let hiz_h = f32(cull_params.z);

    let screen_min = vec2f(
        (ndc_min.x * 0.5 + 0.5) * hiz_w,
        (1.0 - ndc_max.y * 0.5 - 0.5) * hiz_h,
    );
    let screen_max = vec2f(
        (ndc_max.x * 0.5 + 0.5) * hiz_w,
        (1.0 - ndc_min.y * 0.5 - 0.5) * hiz_h,
    );

    // Mip selection: tightest level where the AABB covers ~2-4 texels.
    let rect_size = screen_max - screen_min;
    let max_dim = max(rect_size.x, rect_size.y);
    let mip_level = u32(floor(log2(max(max_dim, 1.0))));
    let mip_clamped = min(mip_level, textureNumLevels(hiz_pyramid) - 1u);

    // Sample Hi-Z at 4 corners of the screen rect
    let mip_scale = 1.0 / f32(1u << mip_clamped);
    let s0 = vec2i(vec2f(screen_min.x, screen_min.y) * mip_scale);
    let s1 = vec2i(vec2f(screen_max.x, screen_min.y) * mip_scale);
    let s2 = vec2i(vec2f(screen_min.x, screen_max.y) * mip_scale);
    let s3 = vec2i(vec2f(screen_max.x, screen_max.y) * mip_scale);

    let d0 = textureLoad(hiz_pyramid, s0, i32(mip_clamped)).r;
    let d1 = textureLoad(hiz_pyramid, s1, i32(mip_clamped)).r;
    let d2 = textureLoad(hiz_pyramid, s2, i32(mip_clamped)).r;
    let d3 = textureLoad(hiz_pyramid, s3, i32(mip_clamped)).r;

    let max_hiz = max(max(d0, d1), max(d2, d3));

    // Depth occlusion test — no bias needed with two-phase approach.
    if min_z_ndc > max_hiz {
        visibility[slot] = 0u;
    } else {
        visibility[slot] = 1u;
    }
}
