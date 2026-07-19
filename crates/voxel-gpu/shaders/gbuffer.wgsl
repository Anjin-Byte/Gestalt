// G-buffer pass. Concatenated *after* traversal.wgsl, so the structure bindings
// (`nodes`@0, `leaf_words`@1, `leaf_bounds`@2) and `traverse_ray` are in scope.
//
// One invocation per pixel: build the camera ray, traverse, and — for a hit —
// derive the surface depth + normal by a ray-box test against the hit voxel's
// unit cube (the differential-validated traversal kernel stays untouched). Writes
// two G-buffer textures the GTAO and composite passes consume:
//   depth  (r32float)   : eye-space linear depth (perpendicular distance along
//                         the view forward); 0 marks a miss (sky).
//   normal (rgba8unorm) : world-space face normal, encoded *0.5+0.5.

struct Camera {
    eye: vec3<f32>,
    tan: f32,
    forward: vec3<f32>,
    aspect: f32,
    right: vec3<f32>,
    n: f32,
    up: vec3<f32>,
    _pad: f32,
    dims: vec4<u32>, // width, height, k, shadow flags (bit0=enabled, bit1=coarse)
}

@group(0) @binding(3) var<uniform> camera: Camera;
@group(0) @binding(4) var gbuf_depth: texture_storage_2d<r32float, write>;
@group(0) @binding(5) var gbuf_normal: texture_storage_2d<rgba8unorm, write>;
// Sun direction (xyz; w pad), animated by the renderer and shared with
// composite.wgsl so the shadow ray traces toward the same sun the direct term
// is lit by. Already unit-length, but normalized again for safety.
@group(0) @binding(6) var<uniform> sun_dir: vec4<f32>;
// Albedo G-buffer target. The hit voxel's per-voxel colour is produced by
// `fetch_albedo` (defined in the concatenated lookup module: color_lookup.wgsl for
// truecolor, palette_lookup.wgsl for palette — each declares its own colour/material
// bindings at @8+). composite reads gbuf_albedo gated by dims.w (bit7 truecolor /
// bit2 palette).
@group(0) @binding(7) var gbuf_albedo: texture_storage_2d<rgba8unorm, write>;
// Shadow-ray start offset along the face normal (voxels) — clears the hit voxel
// to avoid self-shadow acne; well under one voxel to avoid contact light-leak.
const SHADOW_BIAS: f32 = 0.03;
// Coarse (brick-level) shadows stop at the occupied 8³ leaf, so the ray must
// start clear of the surface's OWN brick or every surface self-shadows. Skip ~one
// brick along the sun (8³ diagonal ≈ 13.9, but the lit face sits near the brick's
// sun-side edge so ~10 suffices). Trade: contact shadows finer than ~this leak.
const SHADOW_COARSE_SKIP: f32 = 10.0;

// Entry point + face normal of the ray `o + t*d` against the unit voxel cube at
// min corner `vmin`. Returns `vec4(normal.xyz, t_enter)`.
fn voxel_face(o: vec3<f32>, d: vec3<f32>, vmin: vec3<f32>) -> vec4<f32> {
    let inv = vec3<f32>(1.0) / d;
    let t0 = (vmin - o) * inv;
    let t1 = (vmin + vec3<f32>(1.0) - o) * inv;
    let tmin = min(t0, t1);
    let t_near = max(max(tmin.x, tmin.y), tmin.z);
    var nrm: vec3<f32>;
    if (t_near == tmin.x) {
        nrm = vec3<f32>(-sign(d.x), 0.0, 0.0);
    } else if (t_near == tmin.y) {
        nrm = vec3<f32>(0.0, -sign(d.y), 0.0);
    } else {
        nrm = vec3<f32>(0.0, 0.0, -sign(d.z));
    }
    return vec4<f32>(nrm, t_near);
}

// The per-pixel camera ray for this invocation (shared by both g-buffer entries).
fn gbuffer_ray(gid: vec3<u32>) -> vec3<f32> {
    let w = f32(camera.dims.x);
    let h = f32(camera.dims.y);
    let ndc_x = ((f32(gid.x) + 0.5) / w * 2.0 - 1.0) * camera.tan * camera.aspect;
    let ndc_y = (1.0 - (f32(gid.y) + 0.5) / h * 2.0) * camera.tan;
    return normalize(camera.forward + camera.right * ndc_x + camera.up * ndc_y);
}

// Writes depth/normal/shadow/albedo for a g-buffer `hit` — shared by `gbuffer_main`
// (first-occupied `traverse_ray`) and `gbuffer_opaque_main` (skip-transparent
// `traverse_ray_opaque`), so the surface shading is byte-identical regardless of how
// the surface voxel was found.
fn shade_gbuffer_hit(hit: HitResult, dir: vec3<f32>, gid: vec3<u32>) {
    var depth = 0.0; // sky sentinel
    var nrm = vec3<f32>(0.0, 0.0, 1.0);
    var shadow = 1.0; // 1 = lit, 0 = in shadow (stored in normal.a)
    var albedo = vec4<f32>(0.0, 0.0, 0.0, 1.0); // per-voxel baked colour (vec4(0) on miss)
    if (hit.hit == 1u) {
        let vmin = vec3<f32>(f32(hit.world.x), f32(hit.world.y), f32(hit.world.z));
        let face = voxel_face(camera.eye, dir, vmin);
        nrm = face.xyz;
        // Eye-space linear depth = perpendicular distance along forward.
        depth = max(face.w * dot(dir, camera.forward), 1e-4);

        // Ray-traced hard sun shadow: from the lit surface, trace toward the sun;
        // any hit before leaving the grid means occluded. Only for sun-facing
        // surfaces (back-facing get no direct sun anyway). Offset along the face
        // normal so the ray starts outside the hit voxel (no self-shadow acne).
        // dims.w packs the shadow flags: bit0 = shadows enabled, bit1 = coarse
        // (brick-level traverse_occluded) vs fine (full traverse_ray).
        let shadows_on = (camera.dims.w & 1u) != 0u;
        let coarse = (camera.dims.w & 2u) != 0u;
        let sun = normalize(sun_dir.xyz);
        if (shadows_on && dot(nrm, sun) > 0.0) {
            let world_pos = camera.eye + dir * face.w;
            var occ: u32;
            if (coarse) {
                // Start a brick clear of the surface so it doesn't self-shadow
                // against its own occupied leaf (brick-level stop can't resolve it).
                let origin = world_pos + nrm * SHADOW_BIAS + sun * SHADOW_COARSE_SKIP;
                occ = traverse_occluded(origin, sun, camera.n, camera.dims.z).w;
            } else {
                let origin = world_pos + nrm * SHADOW_BIAS;
                occ = traverse_ray(origin, sun, camera.n, camera.dims.z).hit;
            }
            if (occ == 1u) {
                shadow = 0.0;
            }
        }

        // Per-voxel albedo at the hit — truecolor or palette, depending on which
        // lookup module is concatenated. composite gates this on dims.w (bit7/bit2).
        albedo = fetch_albedo(hit.leaf, hit.vox, hit.world);
    }

    let coord = vec2<u32>(gid.x, gid.y);
    textureStore(gbuf_depth, coord, vec4<f32>(depth, 0.0, 0.0, 0.0));
    textureStore(gbuf_normal, coord, vec4<f32>(nrm * 0.5 + 0.5, shadow));
    textureStore(gbuf_albedo, coord, albedo);
}

@compute @workgroup_size(8, 8)
fn gbuffer_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= camera.dims.x || gid.y >= camera.dims.y) {
        return;
    }
    let dir = gbuffer_ray(gid);
    let hit = traverse_ray(camera.eye, dir, camera.n, camera.dims.z);
    shade_gbuffer_hit(hit, dir, gid);
}
