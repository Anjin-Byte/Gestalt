// R-2: Depth Prepass — vertex-only, no color output.
// Populates depth buffer for early-Z rejection in R-5 and downstream stages.

struct Camera {
    view_proj: mat4x4f,
    position: vec4f,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> vertex_pool: array<u32>;

@vertex
fn vs_depth(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
    let base = vi * 4u;
    let px = bitcast<f32>(vertex_pool[base]);
    let py = bitcast<f32>(vertex_pool[base + 1u]);
    let pz = bitcast<f32>(vertex_pool[base + 2u]);
    return camera.view_proj * vec4f(px, py, pz, 1.0);
}
