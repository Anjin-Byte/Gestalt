// GI Debug Visualizations — fullscreen passes reading cascade atlas textures.
//
// Entry points:
//   fs_cascade_raw   — raw atlas A contents (RGB + opacity tint)
//   fs_gi_only       — GI indirect term with high exposure
//   fs_opacity_map   — alpha channel only (white=hit, black=miss)
//   fs_atlas_b       — raw atlas B contents (ping-pong partner)
//
// The depth-reconstructed normal viz uses a separate binding (depth texture).

@group(0) @binding(0) var cascade_tex_a: texture_2d<f32>;
@group(0) @binding(1) var cascade_tex_b: texture_2d<f32>;
@group(0) @binding(2) var debug_depth_tex: texture_depth_2d;
@group(0) @binding(3) var<uniform> debug_camera: DebugCamera;
@group(0) @binding(4) var normal_gbuffer: texture_2d<f32>;

struct DebugCamera {
    proj_inv: mat4x4f,
    view_inv: mat4x4f,
};

const BASE_DIRS: u32 = 1u;  // must match cascade_build.wgsl

fn octahedral_decode(uv: vec2f) -> vec3f {
    let f = uv * 2.0 - 1.0;
    var n = vec3f(f.x, 1.0 - abs(f.x) - abs(f.y), f.y);
    let t = max(-n.y, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.z += select(t, -t, n.z >= 0.0);
    return normalize(n);
}

// Reconstruct GI at a screen pixel: cosine-weighted sum over cascade 0 directions
// with bilateral depth-weighted probe interpolation.
fn reconstruct_gi(atlas: texture_2d<f32>, pixel: vec2f, normal: vec3f) -> vec4f {
    let atlas_dims = vec2i(textureDimensions(atlas));
    let probe_grid = atlas_dims / i32(BASE_DIRS);

    let frag_depth = textureLoad(debug_depth_tex, vec2i(pixel), 0);

    let probe_f = pixel / f32(BASE_DIRS) - 0.5;
    let base_probe = vec2i(floor(probe_f));
    let frac = fract(probe_f);
    let max_probe = probe_grid - vec2i(1);
    let spacing = i32(BASE_DIRS);

    // Bilateral depth weights for 4 neighbor probes
    var bw: array<f32, 4>;
    var sw_total = 0.0;
    for (var i = 0u; i < 4u; i++) {
        let dx = i32(i & 1u);
        let dy = i32(i >> 1u);
        let np = clamp(base_probe + vec2i(dx, dy), vec2i(0), max_probe);
        let probe_screen = np * spacing + spacing / 2;
        let nbr_depth = textureLoad(debug_depth_tex, probe_screen, 0);
        let d_diff = abs(frag_depth - nbr_depth);
        let sigma = max(0.05 * frag_depth, 0.001);
        let w_depth = exp(-(d_diff * d_diff) / (2.0 * sigma * sigma));
        let w_spatial = select(1.0 - frac.x, frac.x, dx == 1)
                      * select(1.0 - frac.y, frac.y, dy == 1);
        bw[i] = w_depth * w_spatial;
        sw_total += bw[i];
    }
    if (sw_total > 0.001) {
        for (var i = 0u; i < 4u; i++) { bw[i] /= sw_total; }
    }

    var radiance = vec3f(0.0);
    var opacity = 0.0;
    var total_w = 0.0;

    for (var ddy = 0u; ddy < BASE_DIRS; ddy++) {
        for (var ddx = 0u; ddx < BASE_DIRS; ddx++) {
            let dir_uv = (vec2f(f32(ddx), f32(ddy)) + 0.5) / f32(BASE_DIRS);
            let dir = octahedral_decode(dir_uv);
            let w = max(dot(dir, normal), 0.0);
            if (w < 0.001) { continue; }

            let dir_offset = vec2i(vec2u(ddx, ddy)) * probe_grid;
            var val = vec4f(0.0);
            for (var i = 0u; i < 4u; i++) {
                let dx = i32(i & 1u);
                let dy = i32(i >> 1u);
                let np = clamp(base_probe + vec2i(dx, dy), vec2i(0), max_probe);
                let texel = textureLoad(atlas, dir_offset + np, 0);
                val += bw[i] * texel;
            }

            radiance += w * val.rgb;
            opacity += w * val.a;
            total_w += w;
        }
    }

    if (total_w > 0.001) {
        return vec4f(radiance / total_w, opacity / total_w);
    }
    return vec4f(0.0);
}

struct VsOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> VsOutput {
    var positions = array<vec2f, 3>(
        vec2f(-1.0, -1.0),
        vec2f( 3.0, -1.0),
        vec2f(-1.0,  3.0),
    );
    var uvs = array<vec2f, 3>(
        vec2f(0.0, 1.0),
        vec2f(2.0, 1.0),
        vec2f(0.0, -1.0),
    );

    var out: VsOutput;
    out.position = vec4f(positions[vi], 0.0, 1.0);
    out.uv = uvs[vi];
    return out;
}

// ─── Mode 0x20: Raw Atlas A ─────────────────────────────────────────────
@fragment
fn fs_cascade_raw(@builtin(position) frag_pos: vec4f, @location(0) uv: vec2f) -> @location(0) vec4f {
    let coord = vec2i(frag_pos.xy);
    let val = textureLoad(cascade_tex_a, coord, 0);
    let exposure = 2.0;
    var color = val.rgb * exposure;
    if (val.a > 0.01) {
        color = max(color, vec3f(0.02, 0.01, 0.0));
    }
    return vec4f(clamp(color, vec3f(0.0), vec3f(1.0)), 1.0);
}

// ─── Mode 0x21: Opacity Map ─────────────────────────────────────────────
@fragment
fn fs_opacity_map(@builtin(position) frag_pos: vec4f, @location(0) uv: vec2f) -> @location(0) vec4f {
    let coord = vec2i(frag_pos.xy);
    let val = textureLoad(cascade_tex_a, coord, 0);
    let a = val.a;
    return vec4f(a, a, a, 1.0);
}

// ─── Mode 0x22: GI Only (high exposure) ─────────────────────────────────
@fragment
fn fs_gi_only(@builtin(position) frag_pos: vec4f, @location(0) uv: vec2f) -> @location(0) vec4f {
    let coord = vec2i(frag_pos.xy);
    let val = textureLoad(cascade_tex_a, coord, 0);
    let exposure = 5.0;
    let color = val.rgb * exposure;
    return vec4f(clamp(color, vec3f(0.0), vec3f(1.0)), 1.0);
}

// ─── Mode 0x23: Raw Atlas B ────────────────────────────────────────────
@fragment
fn fs_atlas_b(@builtin(position) frag_pos: vec4f, @location(0) uv: vec2f) -> @location(0) vec4f {
    let coord = vec2i(frag_pos.xy);
    let val = textureLoad(cascade_tex_b, coord, 0);
    let exposure = 2.0;
    var color = val.rgb * exposure;
    if (val.a > 0.01) {
        color = max(color, vec3f(0.02, 0.01, 0.0));
    }
    return vec4f(clamp(color, vec3f(0.0), vec3f(1.0)), 1.0);
}

// ─── Mode 0x24: G-Buffer Normals ────────────────────────────────────────
// Shows the actual vertex normals written by the depth prepass.
// These are the normals the cascade build reads — should be clean per-face colors.
@fragment
fn fs_depth_normals(@builtin(position) frag_pos: vec4f, @location(0) uv: vec2f) -> @location(0) vec4f {
    let coord = vec2i(frag_pos.xy);
    let n = textureLoad(normal_gbuffer, coord, 0).xyz;
    if (dot(n, n) < 0.001) {
        return vec4f(0.0, 0.0, 0.0, 1.0); // no geometry
    }
    // Map [-1,1] to [0,1] for display
    return vec4f(normalize(n) * 0.5 + 0.5, 1.0);
}

// ─── Mode 0x25: World Position Reconstruction ──────────────────────────
// Verifies unproject_to_world: R=x, G=y, B=z mapped to [0,1] via fract.
// If this looks wrong, every cascade ray traces from a wrong origin.
@fragment
fn fs_world_pos(@builtin(position) frag_pos: vec4f, @location(0) uv: vec2f) -> @location(0) vec4f {
    let coord = vec2i(frag_pos.xy);
    let depth = textureLoad(debug_depth_tex, coord, 0);
    if (depth >= 1.0) {
        return vec4f(0.0, 0.0, 0.0, 1.0);
    }

    let tex_dims = vec2f(textureDimensions(debug_depth_tex));
    let screen_uv = (vec2f(frag_pos.xy) + 0.5) / tex_dims;
    let ndc = vec3f(screen_uv * 2.0 - vec2f(1.0), depth);
    let clip = vec4f(ndc.x, -ndc.y, ndc.z, 1.0);
    let view_h = debug_camera.proj_inv * clip;
    let view_pos = view_h.xyz / view_h.w;
    let world_h = debug_camera.view_inv * vec4f(view_pos, 1.0);
    let world_pos = world_h.xyz;

    // Show world position as repeating color bands (fract of position / scale)
    // Each color band repeats every 64 units (one chunk), making spatial errors visible
    let viz = fract(world_pos / 64.0);
    return vec4f(abs(viz), 1.0);
}

// ─── Mode 0x26: Raw Atlas Texel ─────────────────────────────────────────
// Reads cascade_tex_a at screen coordinates with NO reconstruction.
// Shows the raw direction-first tiled atlas. Useful for seeing what the
// cascade actually wrote without any bilinear/cosine smoothing.
@fragment
fn fs_raw_texel(@builtin(position) frag_pos: vec4f, @location(0) uv: vec2f) -> @location(0) vec4f {
    let coord = vec2i(frag_pos.xy);
    let val = textureLoad(cascade_tex_a, coord, 0);
    let exposure = 2.0;
    var color = val.rgb * exposure;
    if (val.a > 0.01) {
        color = max(color, vec3f(0.02, 0.01, 0.0));
    }
    return vec4f(clamp(color, vec3f(0.0), vec3f(1.0)), 1.0);
}

// ─── Mode 0x27: Single Direction Block ──────────────────────────────────
// Shows direction (1,1) from atlas A — an upward-facing direction.
// octahedral_decode(0.375, 0.375) ≈ (-0.24, 0.54, -0.24) — pointing UP.
// If the ceiling emissive panel is working, it should show as a bright patch.
@fragment
fn fs_single_dir(@builtin(position) frag_pos: vec4f, @location(0) uv: vec2f) -> @location(0) vec4f {
    let atlas_dims = vec2i(textureDimensions(cascade_tex_a));
    let probe_grid = atlas_dims / i32(BASE_DIRS);

    // Map screen pixel to probe index
    let probe_idx = vec2i(frag_pos.xy) / i32(BASE_DIRS);
    let clamped = clamp(probe_idx, vec2i(0), probe_grid - vec2i(1));

    // Direction (1,1) block — upward-facing
    let dir_offset = vec2i(1, 1) * probe_grid;
    let val = textureLoad(cascade_tex_a, dir_offset + clamped, 0);
    let exposure = 3.0;
    var color = val.rgb * exposure;
    return vec4f(clamp(color, vec3f(0.0), vec3f(1.0)), 1.0);
}
