// Frustum Pre-Cull: patches indirect_buf to remove off-screen chunks before depth prepass.
//
// Zeros instance_count for chunks outside the frustum. On-screen chunks keep
// their previous instance_count (from last frame's final build_indirect).
// Runs BEFORE the depth prepass.

struct Camera {
    view_proj: mat4x4f,
    position: vec4f,
};

@group(0) @binding(0) var<uniform>            camera:       Camera;
@group(0) @binding(1) var<storage, read>      aabb_buf:     array<vec4f>;
@group(0) @binding(2) var<storage, read>      flags_buf:    array<u32>;
@group(0) @binding(3) var<storage, read_write> indirect_buf: array<u32>;
@group(0) @binding(4) var<uniform>            cull_params:  vec4u;

const IS_EMPTY_BIT: u32 = 1u;
const INDIRECT_STRIDE: u32 = 5u;

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let slot = gid.x;
    if slot >= cull_params.x { return; }

    let ind_instance = slot * INDIRECT_STRIDE + 1u;

    if (flags_buf[slot] & IS_EMPTY_BIT) != 0u {
        indirect_buf[ind_instance] = 0u;
        return;
    }

    let aabb_min = aabb_buf[slot * 2u].xyz;
    let aabb_max = aabb_buf[slot * 2u + 1u].xyz;

    if aabb_min.x >= aabb_max.x || aabb_min.y >= aabb_max.y || aabb_min.z >= aabb_max.z {
        indirect_buf[ind_instance] = 0u;
        return;
    }

    var corners_behind = 0u;
    var ndc_min = vec3f(1e10);
    var ndc_max = vec3f(-1e10);

    for (var i = 0u; i < 8u; i++) {
        let corner = vec4f(
            select(aabb_min.x, aabb_max.x, (i & 1u) != 0u),
            select(aabb_min.y, aabb_max.y, (i & 2u) != 0u),
            select(aabb_min.z, aabb_max.z, (i & 4u) != 0u),
            1.0
        );
        let clip = camera.view_proj * corner;
        if clip.w <= 0.0 { corners_behind += 1u; continue; }
        let ndc = clip.xyz / clip.w;
        ndc_min = min(ndc_min, ndc);
        ndc_max = max(ndc_max, ndc);
    }

    if corners_behind == 8u {
        indirect_buf[ind_instance] = 0u;
        return;
    }

    if corners_behind > 0u {
        if ndc_max.x < -1.0 || ndc_min.x > 1.0 ||
           ndc_max.y < -1.0 || ndc_min.y > 1.0 {
            indirect_buf[ind_instance] = 0u;
        }
        return;
    }

    if ndc_max.x < -1.0 || ndc_min.x > 1.0 ||
       ndc_max.y < -1.0 || ndc_min.y > 1.0 ||
       ndc_max.z < 0.0  || ndc_min.z > 1.0 {
        indirect_buf[ind_instance] = 0u;
        return;
    }

    // On-screen: leave instance_count unchanged
}
