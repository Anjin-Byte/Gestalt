// The themed sky (uniform @10): a vertical gradient between two endpoint
// colours, dithered per pixel so the subtle ramp never bands on rgba8. The
// default contents reproduce the product's original dark-blue gradient, so a
// renderer that never calls set_sky looks as it always did. Colours are sRGB
// channel values in [0, 1] (written raw to the rgba8unorm output, like every
// shade in these kernels).
struct Sky {
    top: vec4<f32>,
    bottom: vec4<f32>,
}
@group(0) @binding(10) var<uniform> sky_env: Sky;

fn sky_color(px: vec2<u32>, height: f32) -> vec4<f32> {
    let t = f32(px.y) / height;
    var c = mix(sky_env.top.rgb, sky_env.bottom.rgb, t);
    // Screen-space hash dither, ±0.5/255: breaks gradient banding without
    // visible noise. Deterministic per pixel, so image differentials hold.
    let n = fract(sin(dot(vec2<f32>(px), vec2<f32>(12.9898, 78.233))) * 43758.5453);
    c += vec3<f32>((n - 0.5) / 255.0);
    return vec4<f32>(c, 1.0);
}
