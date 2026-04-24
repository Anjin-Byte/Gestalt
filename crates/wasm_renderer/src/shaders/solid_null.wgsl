// PBR Solid Shading (null GI backend) — Cook-Torrance BRDF with hemisphere ambient + ACES tone mapping.
// No GI contribution. Group 3 has a dummy uniform binding to satisfy the pipeline layout.
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

// ── Null GI binding (dummy) ──
// A single uniform buffer that's never read — exists only so the
// pipeline layout has a valid group 3.
@group(3) @binding(0) var<uniform> gi_dummy: vec4f;

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

    // ── No GI ──
    let gi_radiance = vec3f(0.0);
    let gi_opacity = 0.0;

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
