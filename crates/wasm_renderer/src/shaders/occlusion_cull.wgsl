// R-4 Phase 1: Chunk-Level Occlusion Cull
//
// One thread per slot. Tests chunk AABB against frustum + Hi-Z pyramid.
// Writes visibility[slot] = 0 (culled) or 1 (visible).
//
// See: docs/Resident Representation/stages/R-4-occlusion-cull.md

struct Camera {
    view_proj: mat4x4f,
    position: vec4f,
};

@group(0) @binding(0) var<uniform>            camera:     Camera;
@group(0) @binding(1) var<storage, read>      aabb_buf:   array<vec4f>;   // 2 vec4f per slot
@group(0) @binding(2) var<storage, read>      flags_buf:  array<u32>;
@group(0) @binding(3) var                     hiz_pyramid: texture_2d<f32>;
@group(0) @binding(4) var<storage, read_write> visibility: array<u32>;
@group(0) @binding(5) var<uniform>            cull_params: vec4u;         // x=slot_count, y=hiz_width, z=hiz_height

const IS_EMPTY_BIT: u32 = 1u;

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let slot = gid.x;
    let slot_count = cull_params.x;
    if slot >= slot_count {
        return;
    }

    // Skip empty chunks
    let flags = flags_buf[slot];
    if (flags & IS_EMPTY_BIT) != 0u {
        visibility[slot] = 0u;
        return;
    }

    let aabb_min = aabb_buf[slot * 2u].xyz;
    let aabb_max = aabb_buf[slot * 2u + 1u].xyz;

    // Degenerate AABB (empty or invalid) → cull
    if aabb_min.x >= aabb_max.x || aabb_min.y >= aabb_max.y || aabb_min.z >= aabb_max.z {
        visibility[slot] = 0u;
        return;
    }

    // Project all 8 AABB corners to clip space
    var ndc_min = vec3f(1e10);
    var ndc_max = vec3f(-1e10);
    var min_z_ndc: f32 = 1e10;
    var any_behind = false;

    for (var i = 0u; i < 8u; i++) {
        let corner = vec4f(
            select(aabb_min.x, aabb_max.x, (i & 1u) != 0u),
            select(aabb_min.y, aabb_max.y, (i & 2u) != 0u),
            select(aabb_min.z, aabb_max.z, (i & 4u) != 0u),
            1.0
        );
        let clip = camera.view_proj * corner;

        if clip.w <= 0.0 {
            any_behind = true;
            continue;
        }

        let ndc = clip.xyz / clip.w;
        ndc_min = min(ndc_min, ndc);
        ndc_max = max(ndc_max, ndc);
        min_z_ndc = min(min_z_ndc, ndc.z);
    }

    // If any corner is behind the camera, conservatively mark visible
    if any_behind {
        visibility[slot] = 1u;
        return;
    }

    // Frustum cull: if entirely outside NDC [-1,1] on any axis
    if ndc_max.x < -1.0 || ndc_min.x > 1.0 ||
       ndc_max.y < -1.0 || ndc_min.y > 1.0 ||
       ndc_max.z < 0.0  || ndc_min.z > 1.0 {
        visibility[slot] = 0u;
        return;
    }

    // Convert NDC → screen coords for Hi-Z lookup
    let hiz_w = f32(cull_params.y);
    let hiz_h = f32(cull_params.z);

    let screen_min = vec2f(
        clamp((ndc_min.x * 0.5 + 0.5) * hiz_w, 0.0, hiz_w - 1.0),
        clamp((1.0 - ndc_max.y * 0.5 - 0.5) * hiz_h, 0.0, hiz_h - 1.0),
    );
    let screen_max = vec2f(
        clamp((ndc_max.x * 0.5 + 0.5) * hiz_w, 0.0, hiz_w - 1.0),
        clamp((1.0 - ndc_min.y * 0.5 - 0.5) * hiz_h, 0.0, hiz_h - 1.0),
    );

    // Select mip level: the level where the rect covers ~1-4 texels
    let rect_size = screen_max - screen_min;
    let max_dim = max(rect_size.x, rect_size.y);
    let mip_level = u32(ceil(log2(max(max_dim, 1.0))));
    let mip_clamped = min(mip_level, textureNumLevels(hiz_pyramid) - 1u);

    // Sample Hi-Z at the 4 corners of the screen rect at the selected mip level
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

    // If the chunk's nearest depth is farther than the farthest known depth
    // in the region it covers, the chunk is fully occluded.
    if min_z_ndc > max_hiz {
        visibility[slot] = 0u;
    } else {
        visibility[slot] = 1u;
    }
}
