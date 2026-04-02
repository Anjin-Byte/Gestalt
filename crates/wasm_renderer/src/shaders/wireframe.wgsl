// R-9: Wireframe — renders edge line segments.
// Uses LineList topology with the wireframe index buffer.

struct Camera {
    view_proj: mat4x4f,
    position: vec4f,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> vertex_pool: array<u32>;

struct VsOutput {
    @builtin(position) clip_pos: vec4f,
};

@vertex
fn vs_wire(@builtin(vertex_index) vi: u32) -> VsOutput {
    let base = vi * 4u;
    let px = bitcast<f32>(vertex_pool[base]);
    let py = bitcast<f32>(vertex_pool[base + 1u]);
    let pz = bitcast<f32>(vertex_pool[base + 2u]);

    var out: VsOutput;
    out.clip_pos = camera.view_proj * vec4f(px, py, pz, 1.0);
    return out;
}

@fragment
fn fs_wire() -> @location(0) vec4f {
    return vec4f(0.2, 0.9, 0.3, 1.0);
}
