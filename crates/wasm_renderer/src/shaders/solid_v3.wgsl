// PBR Solid Shading — Cook-Torrance BRDF with hemisphere ambient + ACES tone mapping.
//
// Vertex data is fetched from a storage buffer (vertex_pool) using vertex_index.
// Material table provides albedo (RGB), roughness, emissive (RGB), opacity as packed f16 pairs.
//
// Vertex format (16 bytes per vertex):
//   vec3f position    (12 bytes)
//   u32   normal_mat  (4 bytes: snorm8x3 normal + u8 material_id)

const PI: f32 = 3.14159265;

struct Camera {
    view_proj: mat4x4f,
    position: vec4f,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> vertex_pool: array<u32>;
@group(2) @binding(0) var<storage, read> material_table: array<vec4u>;

// ── v3 cascade GI bindings ──
//
// Five FRAGMENT-visible bindings on group 3:
//   binding 0: chunk pool's *chunk* slot table (used by v3_chunk_to_chunk_slot
//              in the prepended cascade_common.wgsl)
//   binding 1: chunk pool's slot_table_params uniform
//   binding 2: v3 probe payload (radiance + opacity per direction texel)
//   binding 3: v3 probe_slot_table (chunk_slot → v3 probe_slot, or sentinel)
//   binding 4: V3CascadeParams uniform (grid origin, voxel scale, etc.)
@group(3) @binding(0) var<storage, read> slot_table:           array<u32>;
@group(3) @binding(1) var<uniform>       slot_table_params:    vec4i;
@group(3) @binding(2) var<storage, read> probe_payload:        array<vec4f>;
@group(3) @binding(3) var<storage, read> probe_slot_table:     array<u32>;
@group(3) @binding(4) var<uniform>       cascade_params:       V3CascadeParams;

// ─── Lighting constants ─────────────────────────────────────────────────

const SUN_DIR: vec3f = vec3f(0.242, 0.647, 0.404); // normalize(0.3, 0.8, 0.5)
const SUN_COLOR: vec3f = vec3f(1.0, 0.95, 0.85);
const SUN_INTENSITY: f32 = 3.0;

const SKY_COLOR: vec3f = vec3f(0.0, 0.0, 0.0);
const GROUND_COLOR: vec3f = vec3f(0.0, 0.0, 0.0);
const AMBIENT_INTENSITY: f32 = 0.0;

const F0_DIELECTRIC: vec3f = vec3f(0.04);

// ─── PBR functions ──────────────────────────────────────────────────────

// GGX/Trowbridge-Reitz Normal Distribution Function
fn distribution_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom_term = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom_term * denom_term + 0.0001);
}

// Smith-Schlick Geometry Function (single direction)
fn geometry_schlick(n_dot_x: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    return n_dot_x / (n_dot_x * (1.0 - k) + k);
}

// Smith Geometry (combined for view + light)
fn geometry_smith(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    return geometry_schlick(n_dot_v, roughness) * geometry_schlick(n_dot_l, roughness);
}

// Schlick Fresnel Approximation
fn fresnel_schlick(cos_theta: f32, f0: vec3f) -> vec3f {
    let t = clamp(1.0 - cos_theta, 0.0, 1.0);
    let t2 = t * t;
    let t5 = t2 * t2 * t;
    return f0 + (vec3f(1.0) - f0) * t5;
}

// ACES Filmic Tone Mapping
fn aces_tonemap(x: vec3f) -> vec3f {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3f(0.0), vec3f(1.0));
}

// ─── v3 cascade GI lookup ───────────────────────────────────────────────
//
// Phase A: single-cascade trilinear lookup with shade-time hemisphere
// integration. Mirrors the design in
// `docs/Resident Representation/radiance-cascades-v3-design.md` §3.2.
//
// For a single probe slot, integrate the stored radiance over the upper
// hemisphere of `normal`, weighted by the diffuse cosine factor:
//
//     E(p, n) = sum over directions ω of:
//                   L(p, ω) * max(dot(ω, n), 0)  /  total_weight
//
// where total_weight = sum of max(dot(ω, n), 0) for all stored directions
// (normalizes the discrete sum to the continuous diffuse integral, since
// uniform octahedral sampling has constant solid angle per texel).
//
// Returns vec4(rgb_irradiance, opacity_estimate).
fn v3_sample_probe_irradiance(probe_slot: u32, probe_idx: u32, normal: vec3f) -> vec4f {
    let dirs_per_axis = V3_CASCADE_0_DIRS_PER_AXIS;
    var radiance = vec3f(0.0);
    var opacity = 0.0;
    var total_w = 0.0;
    for (var dy: u32 = 0u; dy < dirs_per_axis; dy = dy + 1u) {
        for (var dx: u32 = 0u; dx < dirs_per_axis; dx = dx + 1u) {
            let dir = v3_oct_decode_texel(dx, dy, dirs_per_axis);
            let cos_t = max(dot(dir, normal), 0.0);
            if (cos_t <= 0.0) { continue; }
            let dir_idx = dx + dy * dirs_per_axis;
            let texel_idx = v3_payload_texel_index(probe_slot, probe_idx, dir_idx);
            let texel = probe_payload[texel_idx];
            radiance = radiance + texel.rgb * cos_t;
            opacity = opacity + texel.a * cos_t;
            total_w = total_w + cos_t;
        }
    }
    if (total_w <= 0.0) {
        return vec4f(0.0);
    }
    return vec4f(radiance / total_w, opacity / total_w);
}

/// Trilinear-interpolate the v3 probe payload at a world voxel position.
/// The hemisphere mask is applied at each probe sample.
///
/// Phase A simplifications:
///   - Looks at the chunk that contains the receiver only (no cross-chunk
///     trilinear neighbors yet — falls back to zero contribution beyond
///     the chunk boundary).
///   - Returns vec4(rgb_irradiance, opacity).
fn v3_sample_irradiance(world_pos: vec3f, normal: vec3f) -> vec4f {
    // Convert world position → world voxel position.
    let world_voxel = (world_pos - cascade_params.grid_origin) / cascade_params.voxel_scale;

    // Find the chunk this voxel belongs to.
    let chunk = v3_world_to_chunk(world_voxel);
    let chunk_slot = v3_chunk_to_chunk_slot(chunk);
    if (chunk_slot == V3_PROBE_SENTINEL) {
        return vec4f(0.0);
    }
    let probe_slot = probe_slot_table[chunk_slot];
    if (probe_slot == V3_PROBE_SENTINEL) {
        return vec4f(0.0);
    }

    // Local probe coordinate (continuous): position relative to the chunk's
    // padded local origin (-1), divided by probe spacing.
    let chunk_origin_world = v3_chunk_world_origin(chunk);
    let local_voxel = world_voxel - chunk_origin_world;
    let probe_f = local_voxel / f32(V3_CASCADE_0_SPACING);

    // 8 surrounding probe corners. Clamp to the valid index range
    // [0, V3_PROBES_PER_CHUNK_AXIS - 1] so trilinear at chunk boundaries
    // doesn't read out of bounds. The clamped corner contributes its own
    // value; cross-chunk trilinear is a Phase B concern.
    let probes_max_idx = i32(V3_PROBES_PER_CHUNK_AXIS) - 1;
    let base_i = vec3i(
        clamp(i32(floor(probe_f.x)), 0, probes_max_idx),
        clamp(i32(floor(probe_f.y)), 0, probes_max_idx),
        clamp(i32(floor(probe_f.z)), 0, probes_max_idx),
    );
    let next_i = vec3i(
        min(base_i.x + 1, probes_max_idx),
        min(base_i.y + 1, probes_max_idx),
        min(base_i.z + 1, probes_max_idx),
    );
    let frac = clamp(probe_f - vec3f(base_i), vec3f(0.0), vec3f(1.0));

    // Trilinear weights (matches gi::v3::reference::trilinear_weights).
    let inv = vec3f(1.0) - frac;
    let w000 = inv.x  * inv.y  * inv.z;
    let w100 = frac.x * inv.y  * inv.z;
    let w010 = inv.x  * frac.y * inv.z;
    let w110 = frac.x * frac.y * inv.z;
    let w001 = inv.x  * inv.y  * frac.z;
    let w101 = frac.x * inv.y  * frac.z;
    let w011 = inv.x  * frac.y * frac.z;
    let w111 = frac.x * frac.y * frac.z;

    // Sample 8 corners, integrating each against the receiver normal.
    var accum = vec4f(0.0);
    accum = accum + w000 * v3_sample_probe_irradiance(probe_slot, v3_probe_index_in_chunk(vec3u(u32(base_i.x), u32(base_i.y), u32(base_i.z))), normal);
    accum = accum + w100 * v3_sample_probe_irradiance(probe_slot, v3_probe_index_in_chunk(vec3u(u32(next_i.x), u32(base_i.y), u32(base_i.z))), normal);
    accum = accum + w010 * v3_sample_probe_irradiance(probe_slot, v3_probe_index_in_chunk(vec3u(u32(base_i.x), u32(next_i.y), u32(base_i.z))), normal);
    accum = accum + w110 * v3_sample_probe_irradiance(probe_slot, v3_probe_index_in_chunk(vec3u(u32(next_i.x), u32(next_i.y), u32(base_i.z))), normal);
    accum = accum + w001 * v3_sample_probe_irradiance(probe_slot, v3_probe_index_in_chunk(vec3u(u32(base_i.x), u32(base_i.y), u32(next_i.z))), normal);
    accum = accum + w101 * v3_sample_probe_irradiance(probe_slot, v3_probe_index_in_chunk(vec3u(u32(next_i.x), u32(base_i.y), u32(next_i.z))), normal);
    accum = accum + w011 * v3_sample_probe_irradiance(probe_slot, v3_probe_index_in_chunk(vec3u(u32(base_i.x), u32(next_i.y), u32(next_i.z))), normal);
    accum = accum + w111 * v3_sample_probe_irradiance(probe_slot, v3_probe_index_in_chunk(vec3u(u32(next_i.x), u32(next_i.y), u32(next_i.z))), normal);

    return accum;
}

// ─── Vertex shader ──────────────────────────────────────────────────────

struct VsOutput {
    @builtin(position) clip_pos: vec4f,
    @location(0) world_normal: vec3f,
    @location(1) @interpolate(flat) material_id: u32,
    @location(2) world_pos: vec3f,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOutput {
    let base = vi * 4u;
    let px = bitcast<f32>(vertex_pool[base]);
    let py = bitcast<f32>(vertex_pool[base + 1u]);
    let pz = bitcast<f32>(vertex_pool[base + 2u]);
    let nm = vertex_pool[base + 3u];

    let pos = vec3f(px, py, pz);

    // Unpack snorm8 normal from bits [23:0]
    let nx_i8 = i32(nm & 0xFFu);
    let ny_i8 = i32((nm >> 8u) & 0xFFu);
    let nz_i8 = i32((nm >> 16u) & 0xFFu);
    let nx = f32(select(nx_i8, nx_i8 - 256, nx_i8 > 127)) / 127.0;
    let ny = f32(select(ny_i8, ny_i8 - 256, ny_i8 > 127)) / 127.0;
    let nz = f32(select(nz_i8, nz_i8 - 256, nz_i8 > 127)) / 127.0;

    let mat_id = (nm >> 24u) & 0xFFu;

    var out: VsOutput;
    out.clip_pos = camera.view_proj * vec4f(pos, 1.0);
    out.world_normal = vec3f(nx, ny, nz);
    out.material_id = mat_id;
    out.world_pos = pos;
    return out;
}

// ─── Fragment shader ────────────────────────────────────────────────────

@fragment
fn fs_main(
    @builtin(position) frag_pos: vec4f,
    @location(0) world_normal: vec3f,
    @location(1) @interpolate(flat) material_id: u32,
    @location(2) world_pos: vec3f,
) -> @location(0) vec4f {
    // ── Unpack material ──
    let entry = material_table[material_id];
    let albedo_rg = unpack2x16float(entry.x);
    let albedo_b_rough = unpack2x16float(entry.y);
    let albedo = vec3f(albedo_rg.x, albedo_rg.y, albedo_b_rough.x);
    let roughness = clamp(albedo_b_rough.y, 0.04, 1.0); // clamp to avoid div-by-zero

    let emissive_rg = unpack2x16float(entry.z);
    let emissive_b_op = unpack2x16float(entry.w);
    let self_emissive = vec3f(emissive_rg.x, emissive_rg.y, emissive_b_op.x);

    // ── Vectors ──
    let N = normalize(world_normal);
    let V = normalize(camera.position.xyz - world_pos);
    let L = SUN_DIR;
    let H = normalize(V + L);

    let NdotL = max(dot(N, L), 0.0);
    let NdotV = max(dot(N, V), 0.001);
    let NdotH = max(dot(N, H), 0.0);
    let HdotV = max(dot(H, V), 0.0);

    // ── Cook-Torrance specular BRDF ──
    let D = distribution_ggx(NdotH, roughness);
    let G = geometry_smith(NdotV, NdotL, roughness);
    let F = fresnel_schlick(HdotV, F0_DIELECTRIC);

    let numerator = D * G * F;
    let denominator = max(4.0 * NdotV * NdotL, 0.001);
    let specular = numerator / denominator;

    // Energy conservation: diffuse is reduced by what Fresnel reflects
    let kD = (vec3f(1.0) - F);
    let diffuse = kD * albedo / PI;

    // ── Direct lighting (sun) ──
    let sun_radiance = SUN_COLOR * SUN_INTENSITY;
    let direct = (diffuse + specular) * sun_radiance * NdotL;

    // ── Hemisphere ambient ──
    let hemisphere = mix(GROUND_COLOR, SKY_COLOR, N.y * 0.5 + 0.5);

    // ── v3 cascade GI lookup ──
    // World-space probe payload, hemisphere-integrated at shade time.
    let gi = v3_sample_irradiance(world_pos, N);
    let gi_radiance = gi.rgb;
    let gi_opacity = gi.a;

    let ao = 1.0 - gi_opacity;

    // Cascade stores outgoing radiance (already /PI at hit surface).
    // Multiply by receiver albedo. Multi-bounce feedback provides energy convergence.
    let indirect = gi_radiance * albedo;

    // When GI provides indirect, it IS the ambient — hemisphere is just a minimal
    // fallback to prevent pure black where cascades have no data (sky pixels, etc.)
    let has_gi = step(0.001, dot(gi_radiance, vec3f(1.0)) + gi_opacity);
    let hemisphere_weight = mix(AMBIENT_INTENSITY, 0.05, has_gi);
    let ambient = hemisphere * albedo * hemisphere_weight * ao + indirect;

    // ── Combine ──
    // AO only modulates ambient (sky fill). Direct sun is not occluded by AO —
    // that requires shadow maps (not implemented). Applying AO to direct light
    // catastrophically darkens enclosed interiors.
    let color = direct + ambient + self_emissive;

    // ── Tone map ──
    // No manual gamma — surface format is sRGB, GPU applies gamma on write.
    let final_color = aces_tonemap(color);

    return vec4f(final_color, 1.0);
}
