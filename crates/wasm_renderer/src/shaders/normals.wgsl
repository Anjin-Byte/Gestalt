// R-9: Normals visualization — maps world-space normals to RGB.

struct Camera {
    view_proj: mat4x4f,
    position: vec4f,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> vertex_pool: array<u32>;

struct VsOutput {
    @builtin(position) clip_pos: vec4f,
    @location(0) world_normal: vec3f,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOutput {
    let base = vi * 4u;
    let px = bitcast<f32>(vertex_pool[base]);
    let py = bitcast<f32>(vertex_pool[base + 1u]);
    let pz = bitcast<f32>(vertex_pool[base + 2u]);
    let nm = vertex_pool[base + 3u];

    let nx_i8 = i32(nm & 0xFFu);
    let ny_i8 = i32((nm >> 8u) & 0xFFu);
    let nz_i8 = i32((nm >> 16u) & 0xFFu);
    let nx = f32(select(nx_i8, nx_i8 - 256, nx_i8 > 127)) / 127.0;
    let ny = f32(select(ny_i8, ny_i8 - 256, ny_i8 > 127)) / 127.0;
    let nz = f32(select(nz_i8, nz_i8 - 256, nz_i8 > 127)) / 127.0;

    var out: VsOutput;
    out.clip_pos = camera.view_proj * vec4f(px, py, pz, 1.0);
    out.world_normal = vec3f(nx, ny, nz);
    return out;
}

@fragment
fn fs_normals(@location(0) world_normal: vec3f) -> @location(0) vec4f {
    let n = normalize(world_normal);
    return vec4f(n * 0.5 + 0.5, 1.0);
}
