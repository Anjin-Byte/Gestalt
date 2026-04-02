// Solid shading — renders chunk geometry with flat directional lighting.
//
// Vertex data is fetched from a storage buffer (vertex_pool) using vertex_index.
// This avoids wgpu vertex buffer layout — the compute shader writes directly
// to the storage buffer and this shader reads it back.
//
// Vertex format (16 bytes per vertex):
//   vec3f position    (12 bytes)
//   u32   normal_mat  (4 bytes: snorm8x3 normal + u8 material_id)

struct Camera {
    view_proj: mat4x4f,
    position: vec4f,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> vertex_pool: array<u32>;
@group(2) @binding(0) var<storage, read> material_table: array<vec4u>;

struct VsOutput {
    @builtin(position) clip_pos: vec4f,
    @location(0) world_normal: vec3f,
    @location(1) @interpolate(flat) material_id: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOutput {
    // Each vertex is 4 u32 words (16 bytes)
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
    // Sign-extend from 8-bit
    let nx = f32(select(nx_i8, nx_i8 - 256, nx_i8 > 127)) / 127.0;
    let ny = f32(select(ny_i8, ny_i8 - 256, ny_i8 > 127)) / 127.0;
    let nz = f32(select(nz_i8, nz_i8 - 256, nz_i8 > 127)) / 127.0;

    let mat_id = (nm >> 24u) & 0xFFu;

    var out: VsOutput;
    out.clip_pos = camera.view_proj * vec4f(pos, 1.0);
    out.world_normal = vec3f(nx, ny, nz);
    out.material_id = mat_id;
    return out;
}

@fragment
fn fs_main(
    @location(0) world_normal: vec3f,
    @location(1) @interpolate(flat) material_id: u32,
) -> @location(0) vec4f {
    // Look up material albedo
    let entry = material_table[material_id];
    // entry.x = albedo_rg (packed f16 pair)
    // entry.y = albedo_b_roughness (packed f16 pair)
    let albedo_rg = unpack2x16float(entry.x);
    let albedo_b_rough = unpack2x16float(entry.y);
    let albedo = vec3f(albedo_rg.x, albedo_rg.y, albedo_b_rough.x);

    // Simple directional light
    let light_dir = normalize(vec3f(0.3, 0.8, 0.5));
    let n = normalize(world_normal);
    let ndotl = max(dot(n, light_dir), 0.0);
    let ambient = 0.15;
    let lit = albedo * (ambient + ndotl * 0.85);

    return vec4f(lit, 1.0);
}
