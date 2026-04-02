// R-9: Depth visualization — fullscreen pass, linearizes depth to grayscale.

struct Camera {
    view_proj: mat4x4f,
    position: vec4f,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var depth_tex: texture_depth_2d;
@group(1) @binding(1) var depth_sampler: sampler;

struct VsOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
};

// Fullscreen triangle — 3 vertices cover the screen
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

@fragment
fn fs_depth_viz(@location(0) uv: vec2f) -> @location(0) vec4f {
    let depth = textureSample(depth_tex, depth_sampler, uv);

    // Linearize: near=0.1, far=2000.0
    let near = 0.1;
    let far = 2000.0;
    let linear = near * far / (far - depth * (far - near));

    // Map to grayscale with contrast boost for nearby geometry
    let t = clamp(linear / 200.0, 0.0, 1.0);
    let gray = 1.0 - t; // closer = brighter

    return vec4f(gray, gray, gray, 1.0);
}
