// R-6: Radiance Cascades v2 — Single-Pass Cast + Inline Merge
//
// Dispatched once per cascade level (highest→lowest). Each dispatch casts rays
// for its interval AND merges with the coarser cascade by reading from atlas_read.
// Uses ping-pong: writes to atlas_write, reads coarser data from atlas_read.
//
// Direction-first atlas layout:
//   atlas_x = dir_x * probe_grid_w + probe_x
//   atlas_y = dir_y * probe_grid_h + probe_y
//
// See: docs/Resident Representation/radiance-cascades-v2.md

// ─── Bindings ───────────────────────────────────────────────────────────

@group(0) @binding(0) var<storage, read> occupancy:         array<u32>;
@group(0) @binding(1) var<storage, read> flags:             array<u32>;
@group(0) @binding(2) var<storage, read> slot_table:        array<u32>;
@group(0) @binding(3) var<storage, read> material_table:    array<vec4u>;
@group(0) @binding(4) var<storage, read> palette:           array<u32>;
@group(0) @binding(5) var<storage, read> palette_meta:      array<u32>;
@group(0) @binding(6) var<storage, read> index_buf_pool:    array<u32>;
@group(0) @binding(7) var<uniform>       slot_table_params: vec4i;

@group(1) @binding(0) var<uniform>       cascade_uniforms:  CascadeUniformData;
@group(1) @binding(1) var                depth_tex:         texture_depth_2d;
@group(1) @binding(2) var                cascade_write:     texture_storage_2d<rgba16float, write>;
@group(1) @binding(3) var                cascade_read:      texture_2d<f32>;
@group(1) @binding(4) var                normal_tex:        texture_2d<f32>;
@group(1) @binding(5) var                cascade_prev:      texture_2d<f32>;

struct CascadeUniformData {
    proj_inv: mat4x4f,
    view_inv: mat4x4f,
    screen_size: vec2u,
    cascade_index: u32,
    frame_index: u32,
    voxel_scale: f32,
    grid_origin: vec3f,
    bounce_intensity: f32,
    prev_view_proj: mat4x4f,
};

const N_CASCADES: u32 = 6u;
const BASE_DIRS: u32 = 1u;       // standard: 1 direction at cascade 0, angular info via merge
const BASE_INTERVAL: f32 = 1.0;  // voxels
const SELF_OFFSET: f32 = 0.01;
const MIN_HIT_DIST: f32 = 0.5;
const SHADOW_MAX_DIST: f32 = 128.0;
const PI: f32 = 3.14159265;

// Sun parameters (must match solid.wgsl)
const SUN_DIR: vec3f = vec3f(0.242, 0.647, 0.404);
const SUN_RADIANCE: vec3f = vec3f(3.0, 2.85, 2.55); // color * intensity
const SKY_COLOR: vec3f = vec3f(0.15, 0.20, 0.35);
const GROUND_COLOR: vec3f = vec3f(0.08, 0.06, 0.04);

// ─── DDA common (prepended at shader creation time) ─────────────────────

// ─── Octahedral decoding ────────────────────────────────────────────────

fn octahedral_decode(uv: vec2f) -> vec3f {
    let f = uv * 2.0 - 1.0;
    var n = vec3f(f.x, 1.0 - abs(f.x) - abs(f.y), f.y);
    let t = max(-n.y, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.z += select(t, -t, n.z >= 0.0);
    return normalize(n);
}

// ─── Helpers ────────────────────────────────────────────────────────────

fn unproject_to_world(screen_uv: vec2f, depth: f32) -> vec3f {
    let ndc = vec3f(screen_uv * 2.0 - vec2f(1.0), depth);
    let clip = vec4f(ndc.x, -ndc.y, ndc.z, 1.0);
    let view_h = cascade_uniforms.proj_inv * clip;
    let view_pos = view_h.xyz / view_h.w;
    let world_h = cascade_uniforms.view_inv * vec4f(view_pos, 1.0);
    return world_h.xyz;
}

fn world_to_voxel(world_pos: vec3f) -> vec3f {
    return (world_pos - cascade_uniforms.grid_origin) / cascade_uniforms.voxel_scale;
}

fn read_normal(screen_x: u32, screen_y: u32) -> vec3f {
    let n = textureLoad(normal_tex, vec2i(i32(screen_x), i32(screen_y)), 0).xyz;
    if (dot(n, n) < 0.001) { return vec3f(0.0, 1.0, 0.0); } // fallback for empty pixels
    return normalize(n);
}

fn sample_prev_indirect(hit_world: vec3f) -> vec3f {
    let clip = cascade_uniforms.prev_view_proj * vec4f(hit_world, 1.0);
    if (clip.w <= 0.0) { return vec3f(0.0); } // behind camera
    let ndc = clip.xyz / clip.w;
    let uv = vec2f(ndc.x * 0.5 + 0.5, -ndc.y * 0.5 + 0.5);
    if (any(uv < vec2f(0.0)) || any(uv >= vec2f(1.0))) { return vec3f(0.0); }
    let texel = vec2i(i32(uv.x * f32(cascade_uniforms.screen_size.x)),
                      i32(uv.y * f32(cascade_uniforms.screen_size.y)));
    return textureLoad(cascade_prev, texel, 0).rgb;
}

fn lookup_emissive(slot: u32, local: vec3u) -> vec3f {
    let meta0 = palette_meta[slot * 2u];
    let bpe = (meta0 >> 16u) & 0xFFu;
    if (bpe == 0u) { return vec3f(0.0); }
    let flat_idx = local.x * DDA_CS_P * DDA_CS_P + local.y * DDA_CS_P + local.z;
    let ib_word_offset = palette_meta[slot * 2u + 1u];
    let bit_offset = flat_idx * bpe;
    let word_idx = ib_word_offset + bit_offset / 32u;
    let bit_pos = bit_offset & 31u;
    let mask = (1u << bpe) - 1u;
    var palette_idx = (index_buf_pool[word_idx] >> bit_pos) & mask;
    if (bit_pos + bpe > 32u) {
        let w1 = index_buf_pool[word_idx + 1u];
        palette_idx = palette_idx | ((w1 & ((1u << (bpe - (32u - bit_pos))) - 1u)) << (32u - bit_pos));
    }
    let pal_word = palette[slot * 128u + palette_idx / 2u];
    let mat_id = select(pal_word & 0xFFFFu, (pal_word >> 16u) & 0xFFFFu, (palette_idx & 1u) != 0u);
    let entry = material_table[mat_id];
    let emissive_rg = unpack2x16float(entry.z);
    let emissive_b_op = unpack2x16float(entry.w);
    return vec3f(emissive_rg.x, emissive_rg.y, emissive_b_op.x);
}

fn lookup_albedo(slot: u32, local: vec3u) -> vec3f {
    let meta0 = palette_meta[slot * 2u];
    let bpe = (meta0 >> 16u) & 0xFFu;
    if (bpe == 0u) { return vec3f(0.5); }
    let flat_idx = local.x * DDA_CS_P * DDA_CS_P + local.y * DDA_CS_P + local.z;
    let ib_word_offset = palette_meta[slot * 2u + 1u];
    let bit_offset = flat_idx * bpe;
    let word_idx = ib_word_offset + bit_offset / 32u;
    let bit_pos = bit_offset & 31u;
    let mask = (1u << bpe) - 1u;
    var palette_idx = (index_buf_pool[word_idx] >> bit_pos) & mask;
    if (bit_pos + bpe > 32u) {
        let w1 = index_buf_pool[word_idx + 1u];
        palette_idx = palette_idx | ((w1 & ((1u << (bpe - (32u - bit_pos))) - 1u)) << (32u - bit_pos));
    }
    let pal_word = palette[slot * 128u + palette_idx / 2u];
    let mat_id = select(pal_word & 0xFFFFu, (pal_word >> 16u) & 0xFFFFu, (palette_idx & 1u) != 0u);
    let entry = material_table[mat_id];
    let albedo_rg = unpack2x16float(entry.x);
    let albedo_b_rough = unpack2x16float(entry.y);
    return vec3f(albedo_rg.x, albedo_rg.y, albedo_b_rough.x);
}

fn bilateral_weight(d_a: f32, d_b: f32) -> f32 {
    let diff = abs(d_a - d_b);
    let sigma = 0.1 * d_a;
    if (sigma < 0.0001) { return 0.0; }
    return exp(-(diff * diff) / (2.0 * sigma * sigma));
}

// ─── Direction-first atlas addressing ───────────────────────────────────

fn atlas_addr(probe: vec2u, dir: vec2u, probe_grid: vec2u) -> vec2i {
    return vec2i(dir * probe_grid + probe);
}

// ─── Hit radiance: full direct lighting at a voxel surface ──────────────
//
// When a cascade ray hits a voxel, compute the total outgoing radiance:
//   emissive + direct sun (with DDA shadow test) + hemisphere ambient
// This is what production implementations sample from the rendered scene.

fn compute_hit_radiance(
    hit_pos: vec3f,
    hit_normal: vec3f,
    albedo: vec3f,
    emissive: vec3f,
) -> vec3f {
    // 1. Emissive always contributes
    var radiance = emissive;

    // 2. Direct sun — only if surface faces the sun
    let ndotl = max(dot(hit_normal, SUN_DIR), 0.0);
    if (ndotl > 0.001) {
        // Shadow test: trace from hit surface toward sun
        // If no geometry blocks the path, surface receives sunlight
        let shadow_origin = hit_pos + hit_normal * 0.5;
        let shadow_hit = dda_trace_first_hit(shadow_origin, SUN_DIR, SHADOW_MAX_DIST);
        if (!shadow_hit.hit) {
            radiance += albedo * SUN_RADIANCE * ndotl / PI;
        }
    }

    // 3. Hemisphere ambient fill (sky above, ground below)
    let hemisphere = mix(GROUND_COLOR, SKY_COLOR, hit_normal.y * 0.5 + 0.5);
    radiance += hemisphere * albedo * 0.5;

    // 4. Multi-bounce: sample previous frame's converged cascade at this hit point.
    //    Each frame adds one more bounce; converges via albedo < 1 geometric series.
    let hit_world = hit_pos * cascade_uniforms.voxel_scale + cascade_uniforms.grid_origin;
    let indirect = sample_prev_indirect(hit_world);
    radiance += indirect * albedo * cascade_uniforms.bounce_intensity;

    return radiance;
}

// ─── Main ───────────────────────────────────────────────────────────────

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let ci = cascade_uniforms.cascade_index;
    let sw = cascade_uniforms.screen_size.x;
    let sh = cascade_uniforms.screen_size.y;

    // ── GI disabled: clear atlas ──
    if (ci == 0xFFFFFFFFu) {
        if (gid.x < sw && gid.y < sh) {
            textureStore(cascade_write, vec2i(gid.xy), vec4f(0.0));
        }
        return;
    }

    // ── Decode thread → (probe, direction) ──
    let dirs_per_axis = BASE_DIRS << ci;
    let probe_grid = vec2u(sw / dirs_per_axis, sh / dirs_per_axis);

    // Direction-first: gid maps across direction blocks of probe_grid size
    let dir_idx = gid.xy / probe_grid;
    let probe_idx = gid.xy % probe_grid;

    if (any(dir_idx >= vec2u(dirs_per_axis)) || any(probe_idx >= probe_grid)) { return; }

    // ── Probe screen position ──
    let probe_spacing = dirs_per_axis;
    let screen_x = probe_idx.x * probe_spacing + probe_spacing / 2u;
    let screen_y = probe_idx.y * probe_spacing + probe_spacing / 2u;

    // ── Depth + world position ──
    let depth = textureLoad(depth_tex, vec2i(i32(screen_x), i32(screen_y)), 0);
    if (depth >= 1.0) {
        textureStore(cascade_write, vec2i(gid.xy), vec4f(0.0));
        return;
    }

    let screen_uv = (vec2f(f32(screen_x), f32(screen_y)) + 0.5) / vec2f(f32(sw), f32(sh));
    let world_pos = unproject_to_world(screen_uv, depth);
    let normal = read_normal(screen_x, screen_y);
    let voxel_pos = world_to_voxel(world_pos);

    // ── Ray direction (octahedral) ──
    let dir_uv = (vec2f(dir_idx) + 0.5) / f32(dirs_per_axis);
    let ray_dir = octahedral_decode(dir_uv);

    // ── Interval for this cascade ──
    // Overlap: extend ray range 15% past the nominal interval end to prevent
    // light leaks at cascade boundaries (GMShaders fix). The highest cascade
    // has no neighbor to overlap with, so it uses the base length.
    let t_start = max(0.5, select(0.0, BASE_INTERVAL * (pow(4.0, f32(ci)) - 1.0) / 3.0, ci > 0u));
    let base_length = BASE_INTERVAL * pow(4.0, f32(ci));
    let t_length = select(base_length, base_length * 1.15, ci < N_CASCADES - 1u);

    // ── Cast ray(s) via DDA ──
    let origin = voxel_pos + normal * SELF_OFFSET;

    var radiance = vec3f(0.0);
    var opacity = 0.0;

    // Pre-averaging: cast N_SUB_RAYS rays at sub-angle offsets and average.
    // At cascade 0 (1 direction), this gives angular diversity that would
    // otherwise require the merge to supply. Higher cascades already have
    // enough directions, so we skip sub-rays there.
    let n_sub = select(1u, 4u, ci == 0u);
    let sub_scale = 0.15; // angular jitter radius (radians)

    for (var si = 0u; si < n_sub; si++) {
        // Generate sub-ray direction via small perturbation
        var sub_dir = ray_dir;
        if (si > 0u) {
            // 4 sub-rays: center + 3 offsets in a cross pattern around ray_dir
            let angle = f32(si) * 2.094395; // 2π/3 spacing
            let perp1 = normalize(cross(ray_dir, select(vec3f(0.0, 1.0, 0.0), vec3f(1.0, 0.0, 0.0), abs(ray_dir.y) > 0.9)));
            let perp2 = cross(ray_dir, perp1);
            sub_dir = normalize(ray_dir + sub_scale * (cos(angle) * perp1 + sin(angle) * perp2));
        }

        let hit = dda_trace_first_hit(origin + sub_dir * t_start, sub_dir, t_length);

        if (hit.hit && hit.t > MIN_HIT_DIST) {
            opacity += 1.0;
            // Use the chunk from the DDA traversal, not re-derived from world coords.
            // dda_world_to_chunk has an off-by-one at chunk boundaries (local y/x/z=62
            // maps to the next chunk), so re-deriving drops materials at edges.
            let slot = dda_chunk_to_slot(hit.chunk);
            if (slot != DDA_SENTINEL) {
                let local = dda_world_to_local(hit.voxel, hit.chunk);
                let emissive = lookup_emissive(slot, local);
                let albedo_hit = lookup_albedo(slot, local);
                let hit_normal = vec3f(hit.face);
                let hit_pos = vec3f(hit.voxel) + 0.5;
                radiance += compute_hit_radiance(hit_pos, hit_normal, albedo_hit, emissive);
            }
        }
    }

    radiance /= f32(n_sub);
    opacity /= f32(n_sub);

    // ── Inline merge with coarser cascade ──
    if (ci < N_CASCADES - 1u && opacity < 0.99) {
        let coarser_ci = ci + 1u;
        let coarser_dirs = BASE_DIRS << coarser_ci;
        let coarser_grid = vec2u(sw / coarser_dirs, sh / coarser_dirs);

        // Map this direction to coarser cascade's direction block
        let coarser_dir_base = dir_idx * 2u;

        // Bilinear interpolation over 4 nearest coarser probes
        let coarser_probe_f = vec2f(probe_idx) / 2.0;
        let base_probe = vec2u(floor(coarser_probe_f));
        let frac = fract(coarser_probe_f);

        var accum = vec4f(0.0);
        var total_w = 0.0;

        for (var dy = 0u; dy < 2u; dy++) {
            for (var dx = 0u; dx < 2u; dx++) {
                let np = base_probe + vec2u(dx, dy);
                if (any(np >= coarser_grid)) { continue; }

                // Bilateral depth weight
                let nbr_spacing = coarser_dirs;
                let nbr_sx = np.x * nbr_spacing + nbr_spacing / 2u;
                let nbr_sy = np.y * nbr_spacing + nbr_spacing / 2u;
                let nbr_depth = textureLoad(depth_tex, vec2i(i32(nbr_sx), i32(nbr_sy)), 0);
                let w_depth = bilateral_weight(depth, nbr_depth);
                if (w_depth < 0.001) { continue; }

                let w_spatial = select(1.0 - frac.x, frac.x, dx == 1u)
                              * select(1.0 - frac.y, frac.y, dy == 1u);
                let w = w_depth * w_spatial;

                // Average 2×2 coarser direction texels
                var coarser_val = vec4f(0.0);
                for (var sdy = 0u; sdy < 2u; sdy++) {
                    for (var sdx = 0u; sdx < 2u; sdx++) {
                        let cd = coarser_dir_base + vec2u(sdx, sdy);
                        let ca = atlas_addr(np, cd, coarser_grid);
                        coarser_val += textureLoad(cascade_read, ca, 0);
                    }
                }
                coarser_val *= 0.25;

                accum += w * coarser_val;
                total_w += w;
            }
        }

        if (total_w > 0.001) {
            let L_coarser = accum.rgb / total_w;
            let o_coarser = accum.a / total_w;
            // Sannikov Eq. 13 — front-to-back "over" operator
            radiance = radiance + (1.0 - opacity) * L_coarser;
            opacity = opacity + (1.0 - opacity) * o_coarser;
        }
    }

    // ── Write to atlas ──
    textureStore(cascade_write, vec2i(gid.xy), vec4f(radiance, opacity));
}
